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
//!   to an in-memory `BTreeMap<Key, Value>` key-value store.
//! - **gRPC transport** ([`network`] + [`server`]) moves Raft RPCs between cluster nodes
//!   using tonic.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use logos::{Command, Key, Value, Raft};
//!
//! // Submit a write through the Raft leader:
//! let cmd = Command::Put {
//!     key: Key::from("my-key"),
//!     value: Value::from(b"my-value".as_slice()),
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

/// An opaque key in the distributed KV store.
///
/// Wraps a `Vec<u8>` to provide type safety — preventing accidental
/// interchange with [`Value`] or other byte vectors. Keys are ordered
/// lexicographically, which enables efficient prefix-based range scans
/// in the underlying `BTreeMap`.
///
/// # Construction
///
/// `Key` can be built from string slices, byte slices, or raw `Vec<u8>`:
///
/// ```
/// use logos::Key;
///
/// let from_str = Key::from("scheduler/etl/last_run");
/// let from_bytes = Key::from(b"scheduler/etl/last_run".as_slice());
/// let from_vec = Key::from(vec![1, 2, 3]);
///
/// assert_eq!(from_str, from_bytes);
/// ```
///
/// # Composite Key Patterns
///
/// Higher-level consumers typically build hierarchical keys using `/`
/// separators, similar to etcd:
///
/// ```
/// use logos::Key;
///
/// fn task_key(workflow: &str, task: &str) -> Key {
///     Key::from(format!("scheduler/{workflow}/{task}"))
/// }
///
/// let key = task_key("etl", "ingest_users");
/// assert_eq!(key.as_bytes(), b"scheduler/etl/ingest_users");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Key(pub Vec<u8>);

impl Key {
    /// Returns a byte-slice view of the key's contents.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Key {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl From<&[u8]> for Key {
    fn from(b: &[u8]) -> Self {
        Self(b.to_vec())
    }
}

impl From<&str> for Key {
    fn from(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }
}

impl From<String> for Key {
    fn from(s: String) -> Self {
        Self(s.into_bytes())
    }
}

/// An opaque value in the distributed KV store.
///
/// Wraps a `Vec<u8>` to distinguish values from [`Key`]s at the type level.
/// Values carry no ordering guarantee — only keys are sorted in the
/// underlying `BTreeMap`.
///
/// # Construction
///
/// `Value` can be built from byte slices, strings, or raw `Vec<u8>`:
///
/// ```
/// use logos::Value;
///
/// let from_bytes = Value::from(42u64.to_be_bytes().as_slice());
/// let from_str = Value::from("hello");
/// let from_vec = Value::from(vec![0xDE, 0xAD]);
///
/// assert_eq!(from_str.as_bytes(), b"hello");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value(pub Vec<u8>);

impl Value {
    /// Returns a byte-slice view of the value's contents.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl From<&[u8]> for Value {
    fn from(b: &[u8]) -> Self {
        Self(b.to_vec())
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self(s.into_bytes())
    }
}

/// A replicated command applied to the distributed KV state machine.
///
/// Commands are proposed by clients (e.g. pneuma) and, once committed by a
/// Raft quorum, are applied to every node's local state machine in log order.
///
/// # Examples
///
/// ```
/// use logos::{Command, Key, Value};
///
/// let put = Command::Put {
///     key: Key::from("scheduler/etl/last_run"),
///     value: Value::from(1234567890u64.to_be_bytes().as_slice()),
/// };
///
/// let delete = Command::Delete {
///     key: Key::from("scheduler/etl/last_run"),
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Insert or update a key-value pair.
    Put { key: Key, value: Value },
    /// Remove a key from the store.
    Delete { key: Key },
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
    Put { prev: Option<Value> },
    /// Response to a [`Command::Delete`]. Contains the removed value.
    Delete { prev: Option<Value> },
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
