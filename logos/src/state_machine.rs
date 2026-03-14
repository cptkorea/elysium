//! # Raft State Machine — Replicated Key-Value Store
//!
//! This module implements [`openraft::RaftStateMachine`] backed by an in-memory
//! `BTreeMap<Key, Value>`. Each committed [`Command`] is applied in log order
//! across every node, guaranteeing linearizable reads through the leader.
//!
//! ## Lifecycle
//!
//! 1. The Raft leader replicates a [`Command`] to a quorum of followers.
//! 2. Once committed, [`StateMachine::apply`] executes the command locally.
//! 3. Snapshots serialize the full map via [`bincode`] for new-node catch-up.
//!
//! ## Example
//!
//! ```rust,ignore
//! use logos::state_machine::{StateMachine, StateMachineData};
//!
//! let mut sm = StateMachine::default();
//! // After applying Command::Put { key: Key::from("k"), value: Value::from("v") }
//! // through Raft, the KV store will contain (Key("k") -> Value("v")).
//! assert!(sm.data().kv.is_empty()); // empty until entries are applied
//! ```

use std::collections::BTreeMap;
use std::io::Cursor;

use openraft::storage::RaftStateMachine;
use openraft::{
    Entry, EntryPayload, LogId, RaftSnapshotBuilder, Snapshot, SnapshotMeta, StorageError,
    StorageIOError, StoredMembership,
};
use serde::{Deserialize, Serialize};

use crate::{Command, CommandResponse, Key, TypeConfig, Value};

/// Converts an arbitrary [`std::error::Error`] into an [`openraft::StorageError`]
/// tagged as a state-machine I/O error.
///
/// This is used throughout snapshot serialization/deserialization where the
/// underlying error type (`bincode::Error`) doesn't map directly to openraft's
/// storage error hierarchy. The error is moved (not borrowed) to satisfy the
/// `'static` bound required by [`openraft::AnyError`].
fn sm_io_err(e: impl std::error::Error + Send + Sync + 'static) -> StorageError<u64> {
    StorageIOError::read_state_machine(openraft::AnyError::new(&e)).into()
}

/// Serializable snapshot of the entire state machine.
///
/// This struct is what gets written to / read from snapshot blobs during
/// leader-to-follower catch-up. It captures:
///
/// - The last applied log entry (so the follower knows where to resume).
/// - The cluster membership at snapshot time.
/// - The full key-value map.
///
/// # Serialization
///
/// Snapshots are serialized with [`bincode`] for compact, fast encoding.
///
/// ```
/// use logos::state_machine::StateMachineData;
///
/// let data = StateMachineData::default();
/// let bytes = bincode::serialize(&data).unwrap();
/// let restored: StateMachineData = bincode::deserialize(&bytes).unwrap();
/// assert!(restored.kv.is_empty());
/// ```
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StateMachineData {
    /// The [`LogId`] of the most recently applied entry, or `None` if no
    /// entries have been applied yet.
    pub last_applied_log: Option<LogId<u64>>,

    /// The cluster membership configuration as of the last applied entry.
    pub last_membership: StoredMembership<u64, openraft::BasicNode>,

    /// The replicated key-value store. [`Key`]s and [`Value`]s are opaque byte
    /// wrappers, leaving interpretation to higher-level consumers (e.g. pneuma
    /// stores `Key("scheduler/{workflow}/{task}") -> Value(timestamp_ms)`).
    pub kv: BTreeMap<Key, Value>,
}

/// The Raft state machine that applies committed log entries to a local
/// key-value store.
///
/// Each node in the cluster maintains its own [`StateMachine`]. Because all
/// nodes apply the same entries in the same order, their `kv` maps converge
/// to identical state.
#[derive(Debug, Default)]
pub struct StateMachine {
    data: StateMachineData,
}

impl StateMachine {
    /// Returns a read-only reference to the underlying [`StateMachineData`].
    ///
    /// Useful for inspecting the current KV state or the last applied log id
    /// outside of the Raft apply path.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let sm = StateMachine::default();
    /// assert!(sm.data().kv.is_empty());
    /// assert!(sm.data().last_applied_log.is_none());
    /// ```
    pub fn data(&self) -> &StateMachineData {
        &self.data
    }
}

impl RaftStateMachine<TypeConfig> for StateMachine {
    type SnapshotBuilder = StateMachineSnapshot;

    /// Returns the last applied log id and the current membership.
    ///
    /// Called by openraft on startup and during leadership transitions to
    /// determine how far this node has progressed.
    async fn applied_state(
        &mut self,
    ) -> Result<
        (
            Option<LogId<u64>>,
            StoredMembership<u64, openraft::BasicNode>,
        ),
        StorageError<u64>,
    > {
        Ok((
            self.data.last_applied_log,
            self.data.last_membership.clone(),
        ))
    }

    /// Applies a batch of committed log entries to the KV store.
    ///
    /// Each entry is processed in order:
    ///
    /// - **Blank** entries (heartbeats / no-ops) produce a no-op response.
    /// - **Normal** entries carry a [`Command`] that mutates the KV map.
    /// - **Membership** entries update the stored cluster membership.
    ///
    /// Returns one [`CommandResponse`] per entry, preserving the previous
    /// value at the affected key (if any) for [`Command::Put`] and
    /// [`Command::Delete`].
    async fn apply<I>(
        &mut self,
        entries: I,
    ) -> Result<Vec<CommandResponse>, StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        let mut responses = Vec::new();

        for entry in entries {
            self.data.last_applied_log = Some(entry.log_id);

            match entry.payload {
                EntryPayload::Blank => {
                    responses.push(CommandResponse::Put { prev: None });
                }
                EntryPayload::Normal(cmd) => match cmd {
                    Command::Put { key, value } => {
                        let prev = self.data.kv.insert(key, value);
                        responses.push(CommandResponse::Put { prev });
                    }
                    Command::Delete { key } => {
                        let prev = self.data.kv.remove(&key);
                        responses.push(CommandResponse::Delete { prev });
                    }
                },
                EntryPayload::Membership(mem) => {
                    self.data.last_membership =
                        StoredMembership::new(Some(entry.log_id), mem);
                    responses.push(CommandResponse::Put { prev: None });
                }
            }
        }

        Ok(responses)
    }

    /// Returns a snapshot builder that captures the current state.
    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        StateMachineSnapshot {
            data: self.data.clone(),
        }
    }

    /// Prepares an empty buffer to receive an incoming snapshot from the leader.
    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<u64>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    /// Installs a snapshot received from the leader, replacing local state.
    ///
    /// The snapshot blob is deserialized with [`bincode`] into
    /// [`StateMachineData`], then the `last_applied_log` and membership are
    /// overwritten from the snapshot metadata to ensure consistency.
    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<u64, openraft::BasicNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<u64>> {
        let data: StateMachineData =
            bincode::deserialize(snapshot.get_ref()).map_err(sm_io_err)?;

        self.data = data;
        self.data.last_applied_log = meta.last_log_id;
        self.data.last_membership = meta.last_membership.clone();

        Ok(())
    }

    /// Returns the current snapshot (if any entries have been applied).
    ///
    /// Used by the leader to send snapshots to lagging followers instead of
    /// replaying the entire log.
    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<u64>> {
        let data = bincode::serialize(&self.data).map_err(sm_io_err)?;

        let last_applied_log = self.data.last_applied_log;
        let snapshot_id = last_applied_log
            .map(|id| format!("{}-{}", id.leader_id, id.index))
            .unwrap_or_default();

        Ok(Some(Snapshot {
            meta: SnapshotMeta {
                last_log_id: last_applied_log,
                last_membership: self.data.last_membership.clone(),
                snapshot_id,
            },
            snapshot: Box::new(Cursor::new(data)),
        }))
    }
}

/// A point-in-time capture of the state machine used to build a snapshot.
///
/// Created by [`StateMachine::get_snapshot_builder`] and consumed by openraft's
/// snapshot machinery. The data is cloned at creation time to avoid holding
/// a lock on the state machine during serialization.
pub struct StateMachineSnapshot {
    data: StateMachineData,
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachineSnapshot {
    /// Serializes the captured state into a [`Snapshot`].
    ///
    /// The resulting blob is a [`bincode`]-encoded [`StateMachineData`] wrapped
    /// in a `Cursor<Vec<u8>>`, suitable for transmission over the gRPC snapshot
    /// RPC.
    async fn build_snapshot(
        &mut self,
    ) -> Result<Snapshot<TypeConfig>, StorageError<u64>> {
        let data = bincode::serialize(&self.data).map_err(sm_io_err)?;

        let last_applied_log = self.data.last_applied_log;
        let snapshot_id = last_applied_log
            .map(|id| format!("{}-{}", id.leader_id, id.index))
            .unwrap_or_default();

        Ok(Snapshot {
            meta: SnapshotMeta {
                last_log_id: last_applied_log,
                last_membership: self.data.last_membership.clone(),
                snapshot_id,
            },
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}
