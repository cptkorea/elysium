use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::RangeBounds;

use openraft::storage::{LogFlushed, RaftLogReader, RaftLogStorage};
use openraft::{Entry, LogId, LogState, StorageError, Vote};

use super::TypeConfig;

/// Raft log storage backed by a BTreeMap, mirroring the sorted-store pattern from ontos.
/// Keys are log indices (`u64`), values are openraft `Entry`s.
#[derive(Debug, Default)]
pub struct LogStore {
    vote: Option<Vote<u64>>,
    committed: Option<LogId<u64>>,
    log: BTreeMap<u64, Entry<TypeConfig>>,
    last_purged: Option<LogId<u64>>,
}

impl RaftLogReader<TypeConfig> for LogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + Send>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry<TypeConfig>>, StorageError<u64>> {
        Ok(self.log.range(range).map(|(_, v)| v.clone()).collect())
    }
}

impl RaftLogStorage<TypeConfig> for LogStore {
    type LogReader = LogStore;

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

    async fn get_log_reader(&mut self) -> Self::LogReader {
        LogStore {
            vote: self.vote.clone(),
            committed: self.committed,
            log: self.log.clone(),
            last_purged: self.last_purged,
        }
    }

    async fn save_vote(&mut self, vote: &Vote<u64>) -> Result<(), StorageError<u64>> {
        self.vote = Some(vote.clone());
        Ok(())
    }

    async fn read_vote(&mut self) -> Result<Option<Vote<u64>>, StorageError<u64>> {
        Ok(self.vote.clone())
    }

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

    async fn truncate(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        let to_remove: Vec<u64> = self
            .log
            .range(log_id.index..)
            .map(|(k, _)| *k)
            .collect();
        for key in to_remove {
            self.log.remove(&key);
        }
        Ok(())
    }

    async fn purge(&mut self, log_id: LogId<u64>) -> Result<(), StorageError<u64>> {
        let to_remove: Vec<u64> = self
            .log
            .range(..=log_id.index)
            .map(|(k, _)| *k)
            .collect();
        for key in to_remove {
            self.log.remove(&key);
        }
        self.last_purged = Some(log_id);
        Ok(())
    }

    async fn save_committed(
        &mut self,
        committed: Option<LogId<u64>>,
    ) -> Result<(), StorageError<u64>> {
        self.committed = committed;
        Ok(())
    }

    async fn read_committed(
        &mut self,
    ) -> Result<Option<LogId<u64>>, StorageError<u64>> {
        Ok(self.committed)
    }
}
