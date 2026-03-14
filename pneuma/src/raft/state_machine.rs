use std::collections::HashMap;
use std::io::Cursor;

use openraft::storage::RaftStateMachine;
use openraft::{
    Entry, EntryPayload, LogId, RaftSnapshotBuilder, Snapshot, SnapshotMeta, StorageError,
    StorageIOError, StoredMembership,
};
use serde::{Deserialize, Serialize};

use super::{Command, CommandResponse, TypeConfig};

fn sm_io_err(e: impl std::error::Error + 'static) -> StorageError<u64> {
    StorageIOError::read_state_machine(openraft::AnyError::new(&e)).into()
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct StateMachineData {
    pub last_applied_log: Option<LogId<u64>>,
    pub last_membership: StoredMembership<u64, openraft::BasicNode>,
    /// (workflow, task) -> last_executed_ms
    pub execution_state: HashMap<(String, String), u64>,
}

#[derive(Debug, Default)]
pub struct StateMachine {
    data: StateMachineData,
}

impl StateMachine {
    pub fn data(&self) -> &StateMachineData {
        &self.data
    }
}

impl RaftStateMachine<TypeConfig> for StateMachine {
    type SnapshotBuilder = StateMachineSnapshot;

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
                    responses.push(CommandResponse { ok: true });
                }
                EntryPayload::Normal(cmd) => {
                    match &cmd {
                        Command::MarkExecuted {
                            workflow,
                            task,
                            timestamp_ms,
                        } => {
                            self.data
                                .execution_state
                                .insert((workflow.clone(), task.clone()), *timestamp_ms);
                        }
                    }
                    responses.push(CommandResponse { ok: true });
                }
                EntryPayload::Membership(mem) => {
                    self.data.last_membership =
                        StoredMembership::new(Some(entry.log_id), mem);
                    responses.push(CommandResponse { ok: true });
                }
            }
        }

        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        StateMachineSnapshot {
            data: self.data.clone(),
        }
    }

    async fn begin_receiving_snapshot(
        &mut self,
    ) -> Result<Box<Cursor<Vec<u8>>>, StorageError<u64>> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta<u64, openraft::BasicNode>,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError<u64>> {
        let data: StateMachineData =
            bincode::deserialize(snapshot.get_ref()).map_err(|e| sm_io_err(&e))?;

        self.data = data;
        self.data.last_applied_log = meta.last_log_id;
        self.data.last_membership = meta.last_membership.clone();

        Ok(())
    }

    async fn get_current_snapshot(
        &mut self,
    ) -> Result<Option<Snapshot<TypeConfig>>, StorageError<u64>> {
        let data =
            bincode::serialize(&self.data).map_err(|e| sm_io_err(&e))?;

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

pub struct StateMachineSnapshot {
    data: StateMachineData,
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachineSnapshot {
    async fn build_snapshot(
        &mut self,
    ) -> Result<Snapshot<TypeConfig>, StorageError<u64>> {
        let data =
            bincode::serialize(&self.data).map_err(|e| sm_io_err(&e))?;

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
