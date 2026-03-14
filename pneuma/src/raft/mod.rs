pub mod log_store;
pub mod network;
pub mod server;
pub mod state_machine;

use std::io::Cursor;

use openraft::BasicNode;
use serde::{Deserialize, Serialize};

/// Replicated command applied to the distributed state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    MarkExecuted {
        workflow: String,
        task: String,
        timestamp_ms: u64,
    },
}

/// Response returned after a command is applied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResponse {
    pub ok: bool,
}

/// Central type configuration that parameterizes all openraft generics.
//
// NOTE: openraft's generic types (e.g. `LogId<NID>`, `Vote<NID>`) use
// `#[serde(bound = "")]` internally to suppress serde's default trait-bound
// inference, which would incorrectly require the config trait itself to be
// serializable rather than just its associated types.
openraft::declare_raft_types!(
    pub TypeConfig:
        D = Command,
        R = CommandResponse,
        NodeId = u64,
        Node = BasicNode,
        Entry = openraft::Entry<TypeConfig>,
        SnapshotData = Cursor<Vec<u8>>,
);

pub type Raft = openraft::Raft<TypeConfig>;
