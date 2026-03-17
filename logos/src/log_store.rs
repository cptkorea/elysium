//! # Raft Log Storage — In-Memory BTreeMap Backend
//!
//! This module implements [`openraft::RaftLogStorage`] using a sorted
//! [`BTreeMap<u64, Entry>`], mirroring the sorted-key architecture used by
//! the ontos LSM-tree. In a future iteration, this will be backed by ontos
//! directly for durable persistence.
//!
//! ## Design
//!
//! The log store maintains:
//!
//! - **vote**: The last persisted Raft vote (survives restarts in a durable impl).
//! - **committed**: The highest log id known to be committed.
//! - **log**: A `BTreeMap<u64, Entry>` mapping log index to entry.
//! - **last_purged**: The log id up to which entries have been compacted away.
//!
//! ## Range Queries
//!
//! Because `BTreeMap` supports efficient range iteration, operations like
//! [`RaftLogReader::try_get_log_entries`] translate directly to `BTreeMap::range`.
//! This mirrors how ontos would serve the same queries via its sorted SSTable
//! structure.
//!
//! ## Example
//!
//! ```rust,ignore
//! use logos::log_store::LogStore;
//!
//! let store = LogStore::default();
//! // The store starts empty—openraft calls `get_log_state` on startup to
//! // discover what (if anything) was previously persisted.
//! ```

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::RangeBounds;

use openraft::storage::{LogFlushed, RaftLogReader, RaftLogStorage};
use openraft::{Entry, LogId, LogState, StorageError, Vote};

use crate::TypeConfig;

/// Raft log storage backed by a [`BTreeMap`].
///
/// Keys are log indices ([`u64`]), values are openraft [`Entry`]s. The sorted
/// nature of `BTreeMap` provides O(log n) point lookups and efficient range
/// scans—the same access pattern that ontos's LSM-tree is optimized for.
///
/// # Thread Safety
///
/// This type is **not** internally synchronized. Openraft ensures that only one
/// task accesses the log store at a time through its internal state machine.
///
/// # Examples
///
/// ```
/// use logos::log_store::LogStore;
///
/// let store = LogStore::default();
/// // openraft will call methods on this store to persist votes, append
/// // entries, and manage log compaction.
/// ```
#[derive(Debug, Default)]
pub struct LogStore {
    /// The last persisted Raft vote. In a durable implementation this would
    /// be fsynced before returning.
    vote: Option<Vote<u64>>,

    /// The highest committed log id, as told by the leader.
    committed: Option<LogId<u64>>,

    /// The actual log entries, keyed by their index.
    log: BTreeMap<u64, Entry<TypeConfig>>,

    /// The log id up to which all entries have been purged (compacted).
    /// Entries at or before this index are no longer available.
    last_purged: Option<LogId<u64>>,
}

impl RaftLogReader<TypeConfig> for LogStore {
    /// Retrieves log entries within the given range.
    ///
    /// Leverages [`BTreeMap::range`] for an efficient O(log n + k) scan where
    /// k is the number of entries returned.
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<u64>> {
        Ok(self.log.range(range).map(|(_, v)| v.clone()).collect())
    }
}

impl RaftLogStorage<TypeConfig> for LogStore {
    type LogReader = LogStore;

    /// Returns the current log boundaries: last purged and last stored log ids.
    ///
    /// Called by openraft on startup to determine where in the log this node
    /// left off. If no entries exist but some were purged, the last purged id
    /// is returned as the upper bound.
    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError<u64>> {
        let last_log_id = self
            .log
            .last_key_value()
            .map(|(_, e)| e.log_id)
            .or(self.last_purged);

        Ok(LogState {
            last_purged_log_id: self.last_purged,
            last_log_id,
        })
    }

    /// Creates a snapshot of the current log store for read-only queries.
    ///
    /// The returned reader is a full clone—acceptable for an in-memory store,
    /// but a durable backend would return a lightweight read-only handle.
    async fn get_log_reader(&mut self) -> Self::LogReader {
        LogStore {
            vote: self.vote.clone(),
            committed: self.committed,
            log: self.log.clone(),
            last_purged: self.last_purged,
        }
    }

    /// Persists a Raft vote.
    ///
    /// In the Raft protocol, a node must persist its vote before responding
    /// to a vote request to prevent double-voting after a crash. In this
    /// in-memory implementation, the value is simply stored; a durable backend
    /// would fsync here.
    async fn save_vote(&mut self, vote: &Vote<u64>) -> Result<(), StorageError<u64>> {
        self.vote = Some(vote.clone());
        Ok(())
    }

    /// Reads the last persisted vote, or `None` if this node has never voted.
    async fn read_vote(&mut self) -> Result<Option<Vote<u64>>, StorageError<u64>> {
        Ok(self.vote.clone())
    }

    /// Appends new entries to the log and signals completion via the callback.
    ///
    /// Entries are inserted by their log index. The callback must be invoked
    /// after the entries are durably persisted (here, "durably" means stored
    /// in the `BTreeMap`). Failing to call the callback will stall Raft
    /// replication.
    ///
    /// # Arguments
    ///
    /// * `entries` — The entries to append, in log-index order.
    /// * `callback` — A [`LogFlushed`] handle that must be completed to
    ///   acknowledge the write to openraft.
    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError<u64>>
    where
        I: IntoIterator<Item = Entry<TypeConfig>> + Send,
        I::IntoIter: Send,
    {
        for entry in entries {
            self.log.insert(entry.log_id.index, entry);
        }
        callback.log_io_completed(Ok(()));
        Ok(())
    }

    /// Removes all log entries from the given index onward (inclusive).
    ///
    /// This is called when a follower receives entries that conflict with its
    /// local log—the follower must truncate its log back to the divergence
    /// point before appending the leader's entries.
    async fn truncate(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        let to_remove: Vec<u64> = self.log.range(log_id.index..).map(|(k, _)| *k).collect();
        for key in to_remove {
            self.log.remove(&key);
        }
        Ok(())
    }

    /// Removes all log entries up to and including the given index.
    ///
    /// Called after a snapshot is taken to reclaim space occupied by entries
    /// that are now represented in the snapshot. The `last_purged` marker is
    /// updated so that [`get_log_state`](Self::get_log_state) reports the
    /// correct lower bound.
    async fn purge(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        let to_remove: Vec<u64> = self.log.range(..=log_id.index).map(|(k, _)| *k).collect();
        for key in to_remove {
            self.log.remove(&key);
        }
        self.last_purged = Some(log_id);
        Ok(())
    }

    /// Persists the committed log id communicated by the leader.
    async fn save_committed(
        &mut self,
        committed: Option<LogId<u64>>,
    ) -> Result<(), StorageError<u64>> {
        self.committed = committed;
        Ok(())
    }

    /// Reads the last persisted committed log id.
    async fn read_committed(&mut self) -> Result<Option<LogId<u64>>, StorageError<u64>> {
        Ok(self.committed)
    }
}
