use openraft::error::{
    InstallSnapshotError, RPCError, RaftError, ReplicationClosed, StreamingError,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    SnapshotResponse, VoteRequest, VoteResponse,
};
use openraft::{BasicNode, Snapshot, Vote};

use super::TypeConfig;

pub mod proto {
    tonic::include_proto!("pneuma.raft");
}

use proto::raft_service_client::RaftServiceClient;
use proto::RaftRequest;

/// Creates network connections to peer Raft nodes.
#[derive(Debug, Default, Clone)]
pub struct NetworkFactory;

impl RaftNetworkFactory<TypeConfig> for NetworkFactory {
    type Network = NetworkClient;

    async fn new_client(&mut self, _target: u64, node: &BasicNode) -> Self::Network {
        NetworkClient {
            addr: node.addr.clone(),
        }
    }
}

/// A tonic gRPC client for a single target Raft node.
pub struct NetworkClient {
    addr: String,
}

impl NetworkClient {
    async fn connect(
        &self,
    ) -> Result<RaftServiceClient<tonic::transport::Channel>, tonic::transport::Error> {
        let url = format!("http://{}", self.addr);
        RaftServiceClient::connect(url).await
    }
}

impl RaftNetwork<TypeConfig> for NetworkClient {
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

        let snap_resp: InstallSnapshotResponse<u64> = bincode::deserialize(&resp.into_inner().data)
            .map_err(|e| RPCError::Network(openraft::error::NetworkError::new(&e)))?;

        Ok(snap_resp)
    }
}
