//! # Raft gRPC Server — Inbound RPC Handler
//!
//! This module implements the server side of the Raft gRPC transport. It
//! receives incoming [`RaftRequest`](crate::network::proto::RaftRequest)
//! messages from peer nodes, deserializes them with [`bincode`], and forwards
//! them to the local [`openraft::Raft`] instance.
//!
//! In addition to the consensus RPCs (vote, append-entries, snapshot), the
//! server exposes a [`Query`](crate::Query) RPC for read-only access to the
//! local state machine. Query reads bypass the Raft log and are served
//! directly from the local replica, providing eventual consistency on
//! follower nodes.
//!
//! ## Request Flow
//!
//! ```text
//!   Peer Node                    This Node
//!   ─────────                    ─────────
//!   NetworkClient::vote() ──► RaftServer::vote()
//!       bincode(VoteReq)         deserialize → raft.vote() → serialize
//!                            ◄── bincode(VoteResp)
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use logos::server::RaftServer;
//! use logos::network::proto::raft_service_server::RaftServiceServer;
//!
//! let raft = Arc::new(/* ... create Raft instance ... */);
//! let sm = logos::state_machine::StateMachine::default();
//! let reader = sm.reader();
//! let svc = RaftServiceServer::new(RaftServer::new(raft, reader));
//!
//! tonic::transport::Server::builder()
//!     .add_service(svc)
//!     .serve("0.0.0.0:5001".parse().unwrap())
//!     .await?;
//! ```

use std::io::Cursor;
use std::sync::Arc;

use openraft::raft::{AppendEntriesRequest, VoteRequest};
use openraft::{Snapshot, SnapshotMeta, Vote};
use tonic::{Request, Response, Status};

use crate::network::proto::raft_service_server::RaftService;
use crate::network::proto::{RaftRequest, RaftResponse};
use crate::{Query, StateReader, TypeConfig};

/// gRPC server that bridges incoming Raft RPCs to the local [`openraft::Raft`]
/// instance and serves read-only [`Query`] requests from the local state
/// machine.
///
/// Each consensus RPC handler follows the same pattern:
/// 1. Deserialize the `bincode`-encoded request from `RaftRequest.data`.
/// 2. Forward to the corresponding `Raft` method.
/// 3. Serialize the response back into `RaftResponse.data`.
///
/// The [`Query`] handler reads directly from the local [`StateReader`]
/// without going through the Raft log, providing eventually-consistent
/// reads suitable for observability and monitoring.
///
/// # Examples
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use logos::server::RaftServer;
///
/// let raft_instance: Arc<logos::Raft> = /* ... */;
/// let reader: logos::StateReader = /* ... from StateMachine::reader() ... */;
/// let server = RaftServer::new(raft_instance, reader);
/// ```
pub struct RaftServer {
    raft: Arc<crate::Raft>,
    reader: StateReader,
}

impl RaftServer {
    /// Creates a new [`RaftServer`] wrapping the given Raft instance and
    /// state machine reader.
    ///
    /// The `Arc` allows the server to be shared across tonic's async tasks.
    /// The [`StateReader`] provides lock-free read access to the local
    /// state machine for serving [`Query`] RPCs.
    pub fn new(raft: Arc<crate::Raft>, reader: StateReader) -> Self {
        Self { raft, reader }
    }
}

#[tonic::async_trait]
impl RaftService for RaftServer {
    /// Handles an incoming vote request from a candidate node.
    ///
    /// Deserializes the [`VoteRequest`], calls [`Raft::vote`](openraft::Raft::vote),
    /// and returns the serialized [`VoteResponse`].
    async fn vote(&self, request: Request<RaftRequest>) -> Result<Response<RaftResponse>, Status> {
        let req: VoteRequest<u64> = bincode::deserialize(&request.into_inner().data)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resp = self
            .raft
            .vote(req)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let data = bincode::serialize(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(RaftResponse { data }))
    }

    /// Handles an incoming append-entries request from the leader.
    ///
    /// This is the core log-replication RPC. The leader sends new entries
    /// (or an empty list as a heartbeat) and the follower appends them to
    /// its local log store.
    async fn append_entries(
        &self,
        request: Request<RaftRequest>,
    ) -> Result<Response<RaftResponse>, Status> {
        let req: AppendEntriesRequest<TypeConfig> =
            bincode::deserialize(&request.into_inner().data)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resp = self
            .raft
            .append_entries(req)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let data = bincode::serialize(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(RaftResponse { data }))
    }

    /// Handles an incoming snapshot from the leader.
    ///
    /// Deserializes the tuple `(Vote, SnapshotMeta, Vec<u8>)` and calls
    /// [`Raft::install_full_snapshot`](openraft::Raft::install_full_snapshot)
    /// to replace the local state machine with the leader's snapshot.
    async fn snapshot(
        &self,
        request: Request<RaftRequest>,
    ) -> Result<Response<RaftResponse>, Status> {
        let (vote, meta, data): (Vote<u64>, SnapshotMeta<u64, openraft::BasicNode>, Vec<u8>) =
            bincode::deserialize(&request.into_inner().data)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let snapshot = Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        };

        let resp = self
            .raft
            .install_full_snapshot(vote, snapshot)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let data = bincode::serialize(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(RaftResponse { data }))
    }

    /// Handles a read-only query against the local state machine.
    ///
    /// Deserializes a [`Query`] from the request, dispatches it to the
    /// [`StateReader`], and returns the serialized [`QueryResponse`].
    /// This bypasses the Raft log entirely — reads are served from
    /// whatever state the local replica has applied so far.
    ///
    /// On follower nodes, the response may be slightly behind the leader.
    /// For linearizable reads, clients should query the leader after
    /// calling `ensure_linearizable`.
    async fn query(&self, request: Request<RaftRequest>) -> Result<Response<RaftResponse>, Status> {
        let q: Query = bincode::deserialize(&request.into_inner().data)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resp = self.reader.query(&q);

        let data = bincode::serialize(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(RaftResponse { data }))
    }
}
