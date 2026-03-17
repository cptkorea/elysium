//! # Pyra — Heartbeat-Driven Task Scheduler
//!
//! [`Pyra`] is pneuma's core scheduler runtime, named after the Aegis from
//! Xenoblade Chronicles 2. It consumes a set of [`ScheduledWorkflow`]
//! definitions, resolves their DAG execution order, and repeatedly evaluates
//! which tasks are due on a fixed one-second heartbeat tick.
//!
//! ## Architecture
//!
//! Workflow definitions live in an immutable [`Snapshot`] behind an
//! [`ArcSwap`](arc_swap::ArcSwap), enabling lock-free reads from the
//! heartbeat loop and atomic swaps when definitions change via
//! [`Pyra::reload`].
//!
//! Tasks can be mutated on request, and its mutable timing is stored
//! in a struct ([`TaskTiming`]) locally by the heartbeat loop. This state
//! is then reconciled when the snapshot version changes and the swap keeps
//! the hot path free of locks and heap allocations.
//!
//! ## Distributed Persistence
//!
//! When a [`logos`] Raft handle is provided (via [`Pyra::with_raft`]), task
//! fire events and workflow definitions are persisted to the replicated KV
//! store. Follower nodes can then serve read-only observability queries
//! with eventual consistency. On startup, the scheduler rehydrates its
//! timing state from logos to avoid re-firing tasks that ran recently on
//! a previous instance.
//!
//! ## Example
//!
//! ```rust,ignore
//! use pneuma::schema::ScheduledWorkflow;
//! use pneuma::scheduler::Pyra;
//!
//! let workflow: ScheduledWorkflow = serde_yml::from_str(yaml_str)?;
//!
//! // Local-only (no distributed persistence):
//! let scheduler = Pyra::new(vec![workflow])?;
//!
//! // With logos Raft persistence:
//! let scheduler = Pyra::with_raft(vec![workflow], raft, reader)?;
//!
//! scheduler.run().await; // blocks forever, ticking every second
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};

use crate::schema::ScheduledWorkflow;
use crate::Error;

/// Immutable per-task configuration extracted from the workflow definition.
///
/// Stored inside [`WorkflowDefinition`] and indexed by DAG node id.
/// This is the portion of task state that never changes between snapshot
/// swaps — only the [`TaskTiming`] (mutable, local) side is updated per tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// How often this task should fire.
    pub interval: Duration,
}

/// Mutable per-task timestamp tracked locally by the heartbeat loop.
///
/// `last_run` uses [`Instant`] (monotonic clock) for elapsed-time checks
/// on the hot path, avoiding issues with clock skew and NTP adjustments.
///
/// The optional `last_fired_epoch_ms` records the wall-clock time of the
/// most recent fire for persistence to the storage engine ([`logos`]).
///
/// These two timestamps represent the same event but use two different
/// clock domains (Instant vs UnixEpoch).
struct TaskTimestamp {
    /// When this task last fired on this node (monotonic), or `None` if never.
    last_run: Option<Instant>,
    /// Wall-clock epoch millis of the last fire, for replication to logos.
    last_fired_epoch_ms: Option<u64>,
}

/// A validated, immutable workflow definition ready for execution.
///
/// To opitmize performance, task names, configs, and execution order are
/// stored in parallel `Vec`s indexed by the same `usize` node indices used
/// internally by the DAG. The `execution_order` vector lists those indices
/// in topological order, so the heartbeat loop can iterate with pure index
/// arithmetic — removing the overhead of string hashing or map lookups on
/// critical paths.
///
/// This struct is part of the immutable [`Snapshot`] and should never be mutated
/// after construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Human-readable name from the YAML definition.
    pub name: String,
    /// Interned task names, indexed by DAG node id.
    pub task_names: Vec<String>,
    /// Topologically sorted DAG node indices.
    pub execution_order: Vec<usize>,
    /// Per-task configuration (intervals), indexed by DAG node id.
    pub task_configs: Vec<TaskConfig>,
}

/// An immutable, atomically swappable snapshot of all workflow definitions.
///
/// The heartbeat loop loads the current snapshot each tick via
/// [`ArcSwap::load`]. When workflows are added or modified,
/// [`Pyra::reload`] builds a new `Snapshot` with an incremented `version`
/// and atomically swaps it in.
///
/// The `version` counter lets the heartbeat loop detect changes and
/// reconcile its local [`TaskTiming`] state with the new definitions.
pub struct Snapshot {
    /// Monotonically increasing counter, bumped on each reload.
    pub version: u64,
    /// Immutable workflow definitions.
    pub workflows: Vec<WorkflowDefinition>,
}

/// The heartbeat-driven task scheduler.
///
/// Named after the Aegis [Pyra](https://xenoblade.fandom.com/wiki/Pyra) from
/// Xenoblade Chronicles 2, who brings fire and life to the party — much like
/// this scheduler brings workflows to life through its heartbeat loop.
///
/// Workflow definitions live in an immutable [`Snapshot`] behind an
/// [`ArcSwap`], enabling lock-free reads from the heartbeat loop and atomic
/// swaps when definitions change. Mutable per-task timing ([`TaskTiming`])
/// is held locally by the heartbeat loop and reconciled on version changes.
///
/// # Construction
///
/// Two constructors are provided:
///
/// - [`Pyra::new`] — local-only, no distributed persistence. Suitable for
///   testing and single-node setups.
/// - [`Pyra::with_raft`] — wires in a [`logos::Raft`] handle and
///   [`logos::StateReader`] for persisting fire events and rehydrating
///   timing state from the replicated KV store.
///
/// ```rust,ignore
/// // Local-only:
/// let scheduler = Pyra::new(vec![workflow1, workflow2])?;
///
/// // With Raft persistence:
/// let scheduler = Pyra::with_raft(vec![workflow1], raft, reader)?;
/// ```
pub struct Pyra {
    /// Immutable workflow snapshot, atomically swappable for dynamic updates.
    snapshot: Arc<ArcSwap<Snapshot>>,
    /// Handle to the logos Raft node for persisting fire events and definitions.
    /// `None` in local-only mode.
    raft: Option<Arc<logos::Raft>>,
    /// Read handle to the logos state machine for rehydrating timing state.
    /// `None` in local-only mode.
    reader: Option<logos::StateReader>,
}

impl Pyra {
    /// Creates a local-only scheduler without distributed persistence.
    ///
    /// Task fire events are not replicated and timing state is not
    /// rehydrated from logos on startup. Suitable for testing and
    /// single-node setups.
    ///
    /// For each workflow, this:
    /// 1. Builds the dependency graph via [`ScheduledWorkflow::to_dag`].
    /// 2. Computes a topological execution order as node indices.
    /// 3. Extracts per-task intervals into a [`TaskConfig`] vec aligned
    ///    with the DAG's node indices.
    ///
    /// Returns an error if any workflow contains an unknown dependency
    /// reference or a cycle.
    pub fn new(workflows: Vec<ScheduledWorkflow>) -> Result<Self, Error> {
        let snapshot = build_snapshot(&workflows, 0)?;
        Ok(Self {
            snapshot: Arc::new(ArcSwap::from(Arc::new(snapshot))),
            raft: None,
            reader: None,
        })
    }

    /// Creates a distributed scheduler with logos Raft persistence.
    ///
    /// In addition to the local scheduling behaviour of [`new`](Self::new),
    /// this constructor wires in a Raft handle for persisting fire events
    /// and a [`StateReader`](logos::StateReader) for rehydrating timing
    /// state from the replicated KV store on startup.
    ///
    /// Task fire timestamps are written to logos asynchronously
    /// (fire-and-forget) so that follower nodes can serve observability
    /// queries. Workflow definitions are persisted on [`reload`](Self::reload).
    pub fn with_raft(
        workflows: Vec<ScheduledWorkflow>,
        raft: Arc<logos::Raft>,
        reader: logos::StateReader,
    ) -> Result<Self, Error> {
        let snapshot = build_snapshot(&workflows, 0)?;
        Ok(Self {
            snapshot: Arc::new(ArcSwap::from(Arc::new(snapshot))),
            raft: Some(raft),
            reader: Some(reader),
        })
    }

    /// Atomically replaces all workflow definitions with a new set.
    ///
    /// Builds a new [`Snapshot`] from the provided workflows with an
    /// incremented version counter, then performs a lock-free pointer
    /// swap via [`ArcSwap::store`]. The heartbeat loop will pick up
    /// the new snapshot on its next tick.
    ///
    /// When a Raft handle is present, the updated definitions are
    /// persisted to logos before the swap so that read replicas see
    /// the new definitions before any fire events reference them.
    ///
    /// # Errors
    ///
    /// Returns an error if any workflow contains an invalid DAG
    /// (unknown dependencies or cycles). The existing snapshot is
    /// left untouched on failure.
    pub async fn reload(&self, workflows: Vec<ScheduledWorkflow>) -> Result<(), Error> {
        let current_version = self.snapshot.load().version;
        let new_snapshot = build_snapshot(&workflows, current_version + 1)?;

        self.persist_definitions(&new_snapshot).await;
        self.snapshot.store(Arc::new(new_snapshot));
        Ok(())
    }

    /// Returns a clone of the shared snapshot handle.
    ///
    /// Useful for passing to other tasks that need read-only access to
    /// workflow definitions (e.g. an observability endpoint).
    pub fn snapshot_handle(&self) -> Arc<ArcSwap<Snapshot>> {
        Arc::clone(&self.snapshot)
    }

    /// Runs the scheduler's heartbeat loop indefinitely.
    ///
    /// Every second, loads the current snapshot (lock-free via
    /// [`ArcSwap::load`]) and iterates over all workflows in topological
    /// order. Tasks whose `interval` has elapsed since their `last_run`
    /// are fired; tasks that have never run fire on the first tick.
    ///
    /// When the snapshot version changes (via [`reload`](Self::reload)),
    /// the local [`TaskTiming`] state is reconciled: surviving tasks
    /// (matched by workflow name + task name) keep their existing timing
    /// and new tasks start with `last_run: None`.
    ///
    /// Each fire event is persisted to logos asynchronously
    /// (fire-and-forget) when a Raft handle is present.
    ///
    /// This method takes `&self` (not `&mut self`) since the snapshot
    /// is read through [`ArcSwap`] and timing state is owned by the
    /// loop. Callers typically spawn it on a dedicated tokio task and
    /// abort the handle to shut down.
    pub async fn run(&self) {
        // TODO: Make this configurable.
        const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut current_snap: Arc<Snapshot> = self.snapshot.load_full();
        let mut timings = self.rehydrate_timings(&current_snap);
        let mut current_version = current_snap.version;

        loop {
            heartbeat.tick().await;

            let snap = self.snapshot.load();
            if snap.version != current_version {
                timings = reconcile_timings(&current_snap, &snap, &timings);
                current_version = snap.version;
                current_snap = self.snapshot.load_full();
            }

            let now = Instant::now();
            let epoch_ms = epoch_millis_now();

            for (wf_idx, wf) in snap.workflows.iter().enumerate() {
                for &idx in &wf.execution_order {
                    let timing = &mut timings[wf_idx][idx];
                    let config = &wf.task_configs[idx];
                    let due = match timing.last_run {
                        None => true,
                        Some(last) => now.duration_since(last) >= config.interval,
                    };

                    if due {
                        println!(
                            "[pneuma] workflow={} task={} status=fired",
                            wf.name, wf.task_names[idx]
                        );
                        timing.last_run = Some(now);
                        timing.last_fired_epoch_ms = Some(epoch_ms);
                        self.persist_fire(&wf.name, &wf.task_names[idx], epoch_ms);
                    }
                }
            }
        }
    }

    /// Persists a task fire timestamp to logos via a fire-and-forget spawn.
    ///
    /// Writes the key `pneuma/wf/{workflow}/task/{task}/last_fired` with
    /// the epoch-millis value as big-endian 8 bytes. Failures are logged
    /// but do not affect the heartbeat loop.
    ///
    /// No-ops silently when no Raft handle is configured (local-only mode).
    fn persist_fire(&self, workflow: &str, task: &str, epoch_ms: u64) {
        let raft = match &self.raft {
            Some(r) => Arc::clone(r),
            None => return,
        };

        let key = format!("pneuma/wf/{workflow}/task/{task}/last_fired");
        tokio::spawn(async move {
            let cmd = logos::Command::Put {
                key: logos::Key::from(key),
                value: logos::Value::from(epoch_ms.to_be_bytes().as_slice()),
            };
            if let Err(e) = raft.client_write(cmd).await {
                eprintln!("[pneuma] failed to persist fire event: {e}");
            }
        });
    }

    /// Persists workflow definitions and version counters to logos.
    ///
    /// Serializes each [`WorkflowDefinition`] with [`bincode`] and writes
    /// it to `pneuma/wf/{name}/def`, along with the snapshot version at
    /// `pneuma/wf/{name}/version`.
    ///
    /// No-ops silently when no Raft handle is configured (local-only mode).
    async fn persist_definitions(&self, snapshot: &Snapshot) {
        let raft = match &self.raft {
            Some(r) => r,
            None => return,
        };

        for wf in &snapshot.workflows {
            if let Ok(data) = bincode::serialize(wf) {
                let cmd = logos::Command::Put {
                    key: logos::Key::from(format!("pneuma/wf/{}/def", wf.name)),
                    value: logos::Value::from(data),
                };
                if let Err(e) = raft.client_write(cmd).await {
                    eprintln!("[pneuma] failed to persist workflow definition: {e}");
                }
            }

            let cmd = logos::Command::Put {
                key: logos::Key::from(format!("pneuma/wf/{}/version", wf.name)),
                value: logos::Value::from(snapshot.version.to_be_bytes().as_slice()),
            };
            if let Err(e) = raft.client_write(cmd).await {
                eprintln!("[pneuma] failed to persist workflow version: {e}");
            }
        }
    }

    /// Reads `last_fired` timestamps from logos and reconstructs local
    /// [`TaskTiming`] state.
    ///
    /// For each task, reads `pneuma/wf/{name}/task/{task}/last_fired`.
    /// If the key exists, converts the epoch-millis value into an
    /// approximate [`Instant`] by computing the offset from the current
    /// wall-clock time. This gives reasonable continuity across restarts —
    /// the first tick won't re-fire tasks that ran recently on a previous
    /// instance.
    ///
    /// Falls back to empty timing (all tasks due immediately) when no
    /// reader is configured or a key is missing.
    fn rehydrate_timings(&self, snapshot: &Snapshot) -> Vec<Vec<TaskTimestamp>> {
        let now = Instant::now();
        let now_epoch_ms = epoch_millis_now();

        let mut all_timings = Vec::with_capacity(snapshot.workflows.len());

        for wf in &snapshot.workflows {
            let mut wf_timings = Vec::with_capacity(wf.task_names.len());
            for task_name in &wf.task_names {
                let timing = self
                    .read_last_fired(&wf.name, task_name)
                    .map(|fired_ms| {
                        let elapsed = Duration::from_millis(now_epoch_ms.saturating_sub(fired_ms));
                        TaskTimestamp {
                            last_run: now.checked_sub(elapsed),
                            last_fired_epoch_ms: Some(fired_ms),
                        }
                    })
                    .unwrap_or(TaskTimestamp {
                        last_run: None,
                        last_fired_epoch_ms: None,
                    });
                wf_timings.push(timing);
            }
            all_timings.push(wf_timings);
        }

        all_timings
    }

    /// Reads the `last_fired` epoch-millis value for a single task from
    /// the logos state machine.
    ///
    /// Returns `None` if no reader is configured, the key is absent, or
    /// the stored value is not exactly 8 bytes.
    fn read_last_fired(&self, workflow: &str, task: &str) -> Option<u64> {
        let reader = self.reader.as_ref()?;
        let key = logos::Key::from(format!("pneuma/wf/{workflow}/task/{task}/last_fired"));
        let value = reader.get(&key)?;
        let bytes: [u8; 8] = value.as_bytes().try_into().ok()?;
        Some(u64::from_be_bytes(bytes))
    }
}

/// Builds an immutable [`Snapshot`] from a set of workflow definitions.
///
/// Validates each workflow's DAG (cycle detection, referential integrity)
/// and computes the topological execution order. Returns the snapshot on
/// success or the first validation error encountered.
fn build_snapshot(workflows: &[ScheduledWorkflow], version: u64) -> Result<Snapshot, Error> {
    let mut definitions = Vec::with_capacity(workflows.len());

    for workflow in workflows {
        let dag = workflow.to_dag()?;
        let order = dag.execution_order_indices()?;

        let index_of: HashMap<&str, usize> = dag
            .nodes()
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i))
            .collect();

        let mut task_configs: Vec<Option<TaskConfig>> = (0..dag.len()).map(|_| None).collect();

        for stage in &workflow.stages {
            for (name, def) in &stage.tasks {
                let idx = index_of[name.as_str()];
                task_configs[idx] = Some(TaskConfig {
                    interval: Duration::from_secs(def.interval_secs),
                });
            }
        }

        definitions.push(WorkflowDefinition {
            name: workflow.name.clone(),
            task_names: dag.into_nodes(),
            execution_order: order,
            task_configs: task_configs
                .into_iter()
                .map(|c| c.expect("all DAG nodes must have a task definition"))
                .collect(),
        });
    }

    Ok(Snapshot {
        version,
        workflows: definitions,
    })
}

/// Reconciles local [`TaskTiming`] state when the snapshot version changes.
///
/// Matches tasks by `(workflow_name, task_name)`: surviving tasks keep
/// their existing timing and new tasks start with `last_run: None`.
/// Removed tasks are silently dropped.
///
/// This function is only called on version changes (very rare — e.g.
/// once every few hours), so the temporary `HashMap` allocation is
/// negligible.
fn reconcile_timings(
    old_snapshot: &Snapshot,
    new_snapshot: &Snapshot,
    old_timings: &[Vec<TaskTimestamp>],
) -> Vec<Vec<TaskTimestamp>> {
    let mut old_by_name: HashMap<(&str, &str), (Option<Instant>, Option<u64>)> = HashMap::new();
    for (wf_idx, wf) in old_snapshot.workflows.iter().enumerate() {
        for (task_idx, task_name) in wf.task_names.iter().enumerate() {
            let t = &old_timings[wf_idx][task_idx];
            old_by_name.insert(
                (wf.name.as_str(), task_name.as_str()),
                (t.last_run, t.last_fired_epoch_ms),
            );
        }
    }

    let mut new_timings = Vec::with_capacity(new_snapshot.workflows.len());
    for wf in &new_snapshot.workflows {
        let mut wf_timings = Vec::with_capacity(wf.task_names.len());
        for task_name in &wf.task_names {
            let lookup = (wf.name.as_str(), task_name.as_str());
            let timing = match old_by_name.get(&lookup) {
                Some(&(last_run, last_fired_epoch_ms)) => TaskTimestamp {
                    last_run,
                    last_fired_epoch_ms,
                },
                None => TaskTimestamp {
                    last_run: None,
                    last_fired_epoch_ms: None,
                },
            };
            wf_timings.push(timing);
        }
        new_timings.push(wf_timings);
    }

    new_timings
}

/// Returns the current wall-clock time as milliseconds since the Unix epoch.
fn epoch_millis_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        // duration_since fails only if the system clock is before 1970;
        // defaults to 0 so observability gets a bogus timestamp rather
        // than a panic.
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ScheduledWorkflow;

    fn parse_workflow(yaml: &str) -> ScheduledWorkflow {
        serde_yml::from_str(yaml).unwrap()
    }

    #[test]
    fn build_snapshot_validates_dag() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: only_stage
    tasks:
      fast_task:
        interval_secs: 1
"#,
        );

        let snap = build_snapshot(&[workflow], 0).unwrap();
        assert_eq!(snap.version, 0);
        assert_eq!(snap.workflows.len(), 1);
        assert_eq!(snap.workflows[0].name, "test");
        assert_eq!(snap.workflows[0].task_names.len(), 1);
        assert_eq!(
            snap.workflows[0].task_configs[0].interval,
            Duration::from_secs(1)
        );
    }

    #[test]
    fn reconcile_preserves_existing_timings() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: stage
    tasks:
      a:
        interval_secs: 10
      b:
        interval_secs: 20
"#,
        );

        let snap = build_snapshot(&[workflow], 0).unwrap();
        let now = Instant::now();

        let old_timings = vec![vec![
            TaskTimestamp {
                last_run: Some(now),
                last_fired_epoch_ms: Some(1000),
            },
            TaskTimestamp {
                last_run: Some(now),
                last_fired_epoch_ms: Some(2000),
            },
        ]];

        let new_timings = reconcile_timings(&snap, &snap, &old_timings);
        assert_eq!(new_timings.len(), 1);
        assert_eq!(new_timings[0].len(), 2);
        assert!(new_timings[0][0].last_run.is_some());
        assert!(new_timings[0][1].last_run.is_some());
        assert_eq!(new_timings[0][0].last_fired_epoch_ms, Some(1000));
        assert_eq!(new_timings[0][1].last_fired_epoch_ms, Some(2000));
    }

    #[test]
    fn reconcile_initializes_new_tasks() {
        let old_workflow = parse_workflow(
            r#"
name: test
stages:
  - name: stage
    tasks:
      a:
        interval_secs: 10
"#,
        );

        let new_workflow = parse_workflow(
            r#"
name: test
stages:
  - name: stage
    tasks:
      a:
        interval_secs: 10
      b:
        interval_secs: 20
"#,
        );

        let old_snap = build_snapshot(&[old_workflow], 0).unwrap();
        let new_snap = build_snapshot(&[new_workflow], 1).unwrap();
        let now = Instant::now();

        let old_timings = vec![vec![TaskTimestamp {
            last_run: Some(now),
            last_fired_epoch_ms: Some(1000),
        }]];

        let new_timings = reconcile_timings(&old_snap, &new_snap, &old_timings);
        assert_eq!(new_timings[0].len(), 2);

        let a_idx = new_snap.workflows[0]
            .task_names
            .iter()
            .position(|n| n == "a")
            .unwrap();
        let b_idx = new_snap.workflows[0]
            .task_names
            .iter()
            .position(|n| n == "b")
            .unwrap();

        assert!(new_timings[0][a_idx].last_run.is_some());
        assert!(new_timings[0][b_idx].last_run.is_none());
    }

    #[tokio::test]
    async fn scheduler_fires_tasks() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: only_stage
    tasks:
      fast_task:
        interval_secs: 1
"#,
        );

        let scheduler = Pyra::new(vec![workflow]).unwrap();
        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tokio::time::sleep(Duration::from_millis(1500)).await;
        handle.abort();
    }

    #[tokio::test]
    async fn reload_updates_snapshot_version() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: stage
    tasks:
      a:
        interval_secs: 10
"#,
        );

        let scheduler = Pyra::new(vec![workflow.clone()]).unwrap();
        assert_eq!(scheduler.snapshot.load().version, 0);

        scheduler.reload(vec![workflow]).await.unwrap();
        assert_eq!(scheduler.snapshot.load().version, 1);
    }
}
