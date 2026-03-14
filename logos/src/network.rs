//! # Raft Network Transport — tonic gRPC Client
//!
//! This module implements [`openraft::RaftNetworkFactory`] and [`openraft::RaftNetwork`]
//! using [tonic](https://docs.rs/tonic) gRPC. Each Raft RPC (vote, append-entries,
//! snapshot) is serialized with [`bincode`] into an opaque `bytes` field on the wire,
//! keeping the protobuf schema simple and decoupled from openraft's internal types.
//!
//! ## Wire Format
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │  RaftRequest { data: bincode(VoteRequest) } │
//! │  ─────────────► gRPC ──────────────►        │
//! │  RaftResponse { data: bincode(VoteResponse)}│
//! └─────────────────────────────────────────────┘
//! ```
//!
//! ## Connection Management
//!
//! [`NetworkFactory`] creates a fresh [`NetworkClient`] per target node. Each
//! client lazily connects on the first RPC call. A production implementation
//! would add connection pooling and retry logic.
//!
//! ## Example
//!
//! ```rust,ignore
//! use logos::network::NetworkFactory;
//! use openraft::BasicNode;
//!
//! let mut factory = NetworkFactory;
//! let client = factory.new_client(1, &BasicNode { addr: "127.0.0.1:5001".into() }).await;
//! ```

use openraft::error::{
    InstallSnapshotError, RPCError, RaftError, ReplicationClosed, StreamingError,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    SnapshotResponse, VoteRequest, VoteResponse,
};
use openraft::{BasicNode, Snapshot, Vote};

use crate::TypeConfig;

/// Generated gRPC stubs from `logos/proto/raft.proto`.
///
/// The proto package is `logos.raft`, yielding this Rust module path. Contains
/// [`RaftRequest`](proto::RaftRequest), [`RaftResponse`](proto::RaftResponse),
/// and the [`RaftServiceClient`](proto::raft_service_client::RaftServiceClient).
pub mod proto {
    tonic::include_proto!("logos.raft");
}

use proto::raft_service_client::RaftServiceClient;
use proto::RaftRequest;

/// Factory that produces [`NetworkClient`] instances for each peer node.
///
/// Implements [`RaftNetworkFactory`] so that openraft can create network
/// connections on demand as cluster membership changes.
///
/// # Examples
///
/// ```rust,ignore
/// use logos::network::NetworkFactory;
///
/// // openraft calls new_client internally when it needs to contact a peer:
/// let mut factory = NetworkFactory;
/// ```
#[derive(Debug, Default, Clone)]
pub struct NetworkFactory;

impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    type Network = NetworkClient;

    /// Creates a new gRPC client targeting the given node's advertised address.
    ///
    /// The address is expected to be in `host:port` format (e.g. `"127.0.0.1:5001"`).
    async fn new_client(&mut self, _target: u64, node: &BasicNode) -> Self::Network {
        NetworkClient {
            addr: node.addr.clone(),
        }
    }
}

/// A tonic gRPC client bound to a single Raft peer.
///
/// Each RPC method:
/// 1. Serializes the openraft request type with [`bincode`].
/// 2. Sends it as a [`RaftRequest`](proto::RaftRequest) message.
/// 3. Deserializes the [`RaftResponse`](proto::RaftResponse) back into the
///    appropriate openraft response type.
pub struct NetworkClient {
    addr: String,
}

impl NetworkClient {
    /// Establishes a gRPC channel to the peer at `self.addr`.
    ///
    /// The URL is constructed as `http://{addr}`. Each call creates a new
    /// connection; a production system would pool these channels.
    async fn connect(
        &self,
    ) -> Result<RaftServiceClient<tonic::transport::Channel>, tonic::transport::Error> {
        let url = format!("http://{}", self.addr);
        RaftServiceClient::connect(url).await
    }
}

impl RaftNetwork<TypeConfig> for NetworkClient {
    /// Sends a Raft vote request to this peer.
    ///
    /// Used during leader election: a candidate asks peers to grant their vote.
    async fn vote(
        &mut self,
        rpc: VoteRequest<u64>,
        _option: RPCOption,
    ) -> Result<VoteResponse<u64>, RPCError<u64, BasicNode, RaftError<u64>>> {
        let data = bincode::serialize(&rpc)
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let mut client = self
            .connect()
            .await
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let resp = client
            .vote(RaftRequest { data })
            .await
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let vote_resp: VoteResponse<u64> = bincode::deserialize(&resp.into_inner().data)
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        Ok(vote_resp)
    }

    /// Sends an append-entries request (log replication) to this peer.
    ///
    /// The leader calls this to replicate new log entries and as a heartbeat
    /// (with an empty entry list) to maintain authority.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<AppendEntriesResponse<u64>, RPCError<u64, BasicNode, RaftError<u64>>> {
        let data = bincode::serialize(&rpc)
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let mut client = self
            .connect()
            .await
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let resp = client
            .append_entries(RaftRequest { data })
            .await
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let ae_resp: AppendEntriesResponse<u64> = bincode::deserialize(&resp.into_inner().data)
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        Ok(ae_resp)
    }

    /// Sends a full snapshot to this peer in a single RPC.
    ///
    /// Used when a follower is too far behind to catch up via log replication
    /// alone. The entire snapshot (vote, metadata, and data) is serialized
    /// into one message and transmitted over the `Snapshot` gRPC method.
    async fn full_snapshot(
        &mut self,
        vote: Vote<u64>,
        snapshot: Snapshot<TypeConfig>,
        _cancel: impl std::future::Future<Output = ReplicationClosed> + Send + 'static,
        _option: RPCOption,
    ) -> Result<SnapshotResponse<u64>, StreamingError<TypeConfig, openraft::error::Fatal<u64>>>
    {
        let payload = (vote, snapshot.meta.clone(), snapshot.snapshot.into_inner());
        let data = bincode::serialize(&payload)
            .map_err(|e| StreamingError::Unreachable(openraft::error::Unreachable::new(&e)))?;

        let mut client = self
            .connect()
            .await
            .map_err(|e| StreamingError::Unreachable(openraft::error::Unreachable::new(&e)))?;

        let resp = client
            .snapshot(RaftRequest { data })
            .await
            .map_err(|e| StreamingError::Unreachable(openraft::error::Unreachable::new(&e)))?;

        let snap_resp: SnapshotResponse<u64> = bincode::deserialize(&resp.into_inner().data)
            .map_err(|e| StreamingError::Unreachable(openraft::error::Unreachable::new(&e)))?;

        Ok(snap_resp)
    }

    /// Sends a chunked snapshot install request to this peer.
    ///
    /// This is the streaming counterpart to [`full_snapshot`](Self::full_snapshot).
    /// Openraft may call this for incremental snapshot transfer, though the
    /// current implementation serializes the entire request into one message.
    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<u64>,
        RPCError<u64, BasicNode, RaftError<u64, InstallSnapshotError>>,
    > {
        let data = bincode::serialize(&rpc)
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let mut client = self
            .connect()
            .await
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let resp = client
            .snapshot(RaftRequest { data })
            .await
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        let snap_resp: InstallSnapshotResponse<u64> =
            bincode::deserialize(&resp.into_inner().data)
                .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        Ok(snap_resp)
    }
}
