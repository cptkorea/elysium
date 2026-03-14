//! # Logos — Distributed Consensus Layer
//!
//! Logos is a distributed key-value store built on the [Raft consensus protocol](https://raft.github.io/)
//! via [`openraft`]. It provides a generic, replicated KV API that any component in the
//! elysium workspace can use for coordinated state.
//!
//! ## Architecture
//!
//! - **Raft consensus** ([`openraft`]) handles leader election and log replication.
//! - **Log storage** ([`log_store::LogStore`]) persists the Raft log in a sorted `BTreeMap`.
//! - **State machine** ([`state_machine::StateMachine`]) applies committed log entries
//!   to an in-memory `BTreeMap<Vec<u8>, Vec<u8>>` key-value store.
//! - **gRPC transport** ([`network`] + [`server`]) moves Raft RPCs between cluster nodes
//!   using tonic.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use logos::{Command, Raft};
//!
//! // Submit a write through the Raft leader:
//! let cmd = Command::Put {
//!     key: b"my-key".to_vec(),
//!     value: b"my-value".to_vec(),
//! };
//! let response = raft.client_write(cmd).await?;
//! ```

use std::io::Cursor;

use openraft::BasicNode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod log_store;
pub mod network;
pub mod server;
pub mod state_machine;

/// A replicated command applied to the distributed KV state machine.
///
/// Commands are proposed by clients (e.g. pneuma) and, once committed by a
/// Raft quorum, are applied to every node's local state machine in log order.
///
/// # Examples
///
/// ```
/// use logos::Command;
///
/// let put = Command::Put {
///     key: b"scheduler/etl/last_run".to_vec(),
///     value: 1234567890u64.to_be_bytes().to_vec(),
/// };
///
/// let delete = Command::Delete {
///     key: b"scheduler/etl/last_run".to_vec(),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Insert or update a key-value pair.
    Put { key: Vec<u8>, value: Vec<u8> },
    /// Remove a key from the store.
    Delete { key: Vec<u8> },
}

/// The response returned after a [`Command`] is applied to the state machine.
///
/// Contains the previous value (if any) that was stored at the affected key,
/// enabling compare-and-swap patterns in higher-level consumers.
///
/// # Examples
///
/// ```
/// use logos::CommandResponse;
///
/// let resp = CommandResponse::Put { prev: None };
/// assert!(matches!(resp, CommandResponse::Put { prev: None }));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandResponse {
    /// Response to a [`Command::Put`]. Contains the previous value at that key.
    Put { prev: Option<Vec<u8>> },
    /// Response to a [`Command::Delete`]. Contains the removed value.
    Delete { prev: Option<Vec<u8>> },
}

// Central type configuration that parameterizes all openraft generics.
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

/// The Raft instance type, parameterized by [`TypeConfig`].
///
/// This is the main entry point for interacting with the consensus layer.
/// Use [`Raft::client_write`](openraft::Raft::client_write) to propose commands
/// and [`Raft::initialize`](openraft::Raft::initialize) to bootstrap a cluster.
pub type Raft = openraft::Raft<TypeConfig>;

/// Errors produced by the logos distributed store.
#[derive(Debug, Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("raft error: {0}")]
    RaftError(String),
}
