use std::io::Cursor;
use std::sync::Arc;

use openraft::raft::{AppendEntriesRequest, VoteRequest};
use openraft::{Snapshot, SnapshotMeta, Vote};
use tonic::{Request, Response, Status};

use super::network::proto::raft_service_server::RaftService;
use super::network::proto::{RaftRequest, RaftResponse};
use super::TypeConfig;

/// gRPC server that forwards incoming Raft RPCs to the local openraft instance.
pub struct RaftServer {
    raft: Arc<super::Raft>,
}

impl RaftServer {
    pub fn new(raft: Arc<super::Raft>) -> Self {
        Self { raft }
    }
}

#[tonic::async_trait]
impl RaftService for RaftServer {
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
