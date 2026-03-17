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
use std::sync::{Arc, RwLock};

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

/// A read-only query against the local KV state machine.
///
/// Queries are served directly from the local replica without going through
/// the Raft log. On follower nodes this means reads may be slightly behind
/// the leader (eventual consistency), which we consider acceptable for
/// observability use cases.
///
/// # Variants
///
/// - [`Get`](Query::Get): Retrieve a single key.
/// - [`Scan`](Query::Scan): Return all key-value pairs whose key starts with
///   a given prefix, leveraging the underlying `BTreeMap`'s lexicographic
///   ordering.
///
/// # Examples
///
/// ```
/// use logos::{Key, Query};
///
/// let get = Query::Get { key: Key::from("scheduler/etl/last_run") };
/// let scan = Query::Scan { prefix: Key::from("scheduler/etl/") };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Query {
    /// Look up a single key in the store.
    Get { key: Key },
    /// Return all entries whose key starts with `prefix`.
    Scan { prefix: Key },
}

/// The response to a [`Query`] against the local KV state machine.
///
/// # Examples
///
/// ```
/// use logos::{QueryResponse, Value};
///
/// let resp = QueryResponse::Get { value: Some(Value::from("hello")) };
/// assert!(matches!(resp, QueryResponse::Get { value: Some(_) }));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryResponse {
    /// The value at the requested key, or `None` if absent.
    Get { value: Option<Value> },
    /// All key-value pairs matching the scan prefix.
    Scan { entries: Vec<(Key, Value)> },
}

/// A read-only handle to the replicated KV state machine.
///
/// Provides shared-access queries against the local state machine data.
/// On follower nodes, reads are eventually consistent — the follower
/// may lag behind the leader by a small number of committed entries.
///
/// Since `StateReader` is `Arc`-backed, it is cheap to clone and safe to share
/// across tasks and threads.
///
/// # Examples
///
/// ```rust,ignore
/// let sm = StateMachine::default();
/// let reader = sm.reader();
///
/// // Read a single key.
/// let val = reader.get(&Key::from("foo"));
///
/// // Prefix scan.
/// let entries = reader.scan_prefix(&Key::from("scheduler/"));
/// ```
#[derive(Debug, Clone)]
pub struct StateReader {
    data: Arc<RwLock<state_machine::StateMachineData>>,
}

impl StateReader {
    /// Retrieves the value for a single key, or `None` if absent.
    pub fn get(&self, key: &Key) -> Option<Value> {
        self.data
            .read()
            .expect("state machine lock poisoned")
            .kv
            .get(key)
            .cloned()
    }

    /// Returns all key-value pairs whose key starts with `prefix`.
    ///
    /// Leverages the `BTreeMap`'s sorted order for efficient prefix
    /// range scans without a full table scan.
    pub fn scan_prefix(&self, prefix: &Key) -> Vec<(Key, Value)> {
        let data = self.data.read().expect("state machine lock poisoned");
        data.kv
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.as_bytes().starts_with(prefix.as_bytes()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Dispatches a [`Query`] and returns the corresponding [`QueryResponse`].
    pub fn query(&self, q: &Query) -> QueryResponse {
        match q {
            Query::Get { key } => QueryResponse::Get {
                value: self.get(key),
            },
            Query::Scan { prefix } => QueryResponse::Scan {
                entries: self.scan_prefix(prefix),
            },
        }
    }
}
