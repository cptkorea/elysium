//! # Raft gRPC Server — Inbound RPC Handler
//!
//! This module implements the server side of the Raft gRPC transport. It
//! receives incoming [`RaftRequest`](crate::network::proto::RaftRequest)
//! messages from peer nodes, deserializes them with [`bincode`], and forwards
//! them to the local [`openraft::Raft`] instance.
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
//! let svc = RaftServiceServer::new(RaftServer::new(raft));
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
use crate::TypeConfig;

/// gRPC server that bridges incoming Raft RPCs to the local [`openraft::Raft`] instance.
///
/// Each RPC handler follows the same pattern:
/// 1. Deserialize the `bincode`-encoded request from `RaftRequest.data`.
/// 2. Forward to the corresponding `Raft` method.
/// 3. Serialize the response back into `RaftResponse.data`.
///
/// # Examples
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use logos::server::RaftServer;
///
/// let raft_instance: Arc<logos::Raft> = /* ... */;
/// let server = RaftServer::new(raft_instance);
/// ```
pub struct RaftServer {
    raft: Arc<crate::Raft>,
}

impl RaftServer {
    /// Creates a new [`RaftServer`] wrapping the given Raft instance.
    ///
    /// The `Arc` allows the server to be shared across tonic's async tasks.
    pub fn new(raft: Arc<crate::Raft>) -> Self {
        Self { raft }
    }
}

#[tonic::async_trait]
impl RaftService for RaftServer {
    /// Handles an incoming vote request from a candidate node.
    ///
    /// Deserializes the [`VoteRequest`], calls [`Raft::vote`](openraft::Raft::vote),
    /// and returns the serialized [`VoteResponse`].
    async fn vote(
        &self,
        request: Request<RaftRequest>,
    ) -> Result<Response<RaftResponse>, Status> {
        let req: VoteRequest<u64> = bincode::deserialize(&request.into_inner().data)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let resp = self
            .raft
            .vote(req)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let data =
            bincode::serialize(&resp).map_err(|e| Status::internal(e.to_string()))?;

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

        let data =
            bincode::serialize(&resp).map_err(|e| Status::internal(e.to_string()))?;

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

        let data =
            bincode::serialize(&resp).map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(RaftResponse { data }))
    }
}
