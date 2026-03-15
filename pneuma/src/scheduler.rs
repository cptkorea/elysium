//! # Pyra — Heartbeat-Driven Task Scheduler
//!
//! [`Pyra`] is pneuma's core scheduler runtime, named after the Aegis from
//! Xenoblade Chronicles 2. It consumes a set of [`ScheduledWorkflow`]
//! definitions, resolves their DAG execution order, and repeatedly evaluates
//! which tasks are due on a fixed one-second heartbeat tick.
//!
//! ## Design
//!
//! - **Heartbeat loop**: A `tokio::time::interval` fires every second.
//!   Each tick iterates over all workflows and checks whether each task's
//!   elapsed time exceeds its configured `interval_secs`.
//!
//! - **Execution order**: Tasks are visited in the topological order
//!   produced by [`DirectedAcyclicGraph::execution_order_indices`](elysium_common::dag::DirectedAcyclicGraph::execution_order_indices),
//!   ensuring that dependencies are evaluated (and eventually executed)
//!   before their dependents.
//!
//! - **Local-only state**: Task timing is tracked via [`Instant`], a
//!   monotonic clock handle that cannot be serialized. This is intentional —
//!   local scheduling state is ephemeral. Durable execution history will be
//!   persisted to [`logos`] once the distributed integration is wired in,
//!   and the scheduler will rehydrate from it on startup.
//!
//! ## Example
//!
//! ```rust,ignore
//! use pneuma::schema::ScheduledWorkflow;
//! use pneuma::scheduler::Pyra;
//!
//! let workflow: ScheduledWorkflow = serde_yml::from_str(yaml_str)?;
//! let mut scheduler = Pyra::new(vec![workflow])?;
//! scheduler.run().await; // blocks forever, ticking every second
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::schema::ScheduledWorkflow;
use crate::Error;

/// Per-task timing state tracked locally by the scheduler.
///
/// `last_run` uses [`Instant`] (monotonic clock) rather than wall-clock time
/// because the scheduler only needs to measure elapsed duration, not absolute
/// timestamps. This avoids issues with clock skew and NTP adjustments.
///
/// When distributed execution tracking is added, a separate `last_executed_ms`
/// (epoch millis) will be written to logos as the durable source of truth.
struct TaskState {
    /// How often this task should fire.
    interval: Duration,
    /// When this task last fired on this node, or `None` if it hasn't yet.
    last_run: Option<Instant>,
}

/// The heartbeat-driven task scheduler.
///
/// Named after the Aegis [Pyra](https://xenoblade.fandom.com/wiki/Pyra) from
/// Xenoblade Chronicles 2, who brings fire and life to the party — much like
/// this scheduler brings workflows to life through its heartbeat loop.
///
/// Holds a set of resolved workflows and runs an infinite async loop that
/// checks and fires due tasks every second. Currently, "firing" is a stub
/// that logs to stdout; real execution dispatch will be added later.
///
/// # Construction
///
/// Built from a list of [`ScheduledWorkflow`] definitions. Construction
/// validates the DAG (cycle detection, referential integrity) and computes
/// the topological execution order for each workflow.
///
/// ```rust,ignore
/// let scheduler = Pyra::new(vec![workflow1, workflow2])?;
/// ```
pub struct Pyra {
    workflows: Vec<ResolvedWorkflow>,
}

/// A workflow whose DAG has been validated and flattened into a linear
/// execution order.
///
/// Task names and timing state are stored in parallel `Vec`s indexed by
/// the same `usize` node indices used internally by the DAG. The
/// `execution_order` vector lists those indices in topological order,
/// so the heartbeat loop can iterate with pure index arithmetic — no
/// string hashing or map lookups on the hot path.
struct ResolvedWorkflow {
    name: String,
    /// Interned task names, indexed by DAG node id.
    task_names: Vec<String>,
    /// Topologically sorted DAG node indices.
    execution_order: Vec<usize>,
    /// Per-task timing state, indexed by DAG node id.
    tasks: Vec<TaskState>,
}

impl Pyra {
    /// Creates a new scheduler from a list of workflow definitions.
    ///
    /// For each workflow, this:
    /// 1. Builds the dependency graph via [`ScheduledWorkflow::to_dag`].
    /// 2. Computes a topological execution order as node indices.
    /// 3. Extracts per-task intervals into a `Vec<TaskState>` aligned with
    ///    the DAG's node indices.
    ///
    /// Returns an error if any workflow contains an unknown dependency
    /// reference or a cycle.
    pub fn new(workflows: Vec<ScheduledWorkflow>) -> Result<Self, Error> {
        let mut resolved = Vec::new();

        for workflow in &workflows {
            let dag = workflow.to_dag()?;
            let order = dag.execution_order_indices()?;

            let index_of: HashMap<&str, usize> = dag
                .nodes()
                .iter()
                .enumerate()
                .map(|(i, name)| (name.as_str(), i))
                .collect();

            let mut tasks: Vec<Option<TaskState>> = (0..dag.len()).map(|_| None).collect();
            for stage in &workflow.stages {
                for (name, def) in &stage.tasks {
                    let idx = index_of[name.as_str()];
                    tasks[idx] = Some(TaskState {
                        interval: Duration::from_secs(def.interval_secs),
                        last_run: None,
                    });
                }
            }

            resolved.push(ResolvedWorkflow {
                name: workflow.name.clone(),
                task_names: dag.into_nodes(),
                execution_order: order,
                tasks: tasks.into_iter().map(|t| t.expect("all DAG nodes must have a task definition")).collect(),
            });
        }

        Ok(Self {
            workflows: resolved,
        })
    }

    /// Runs the scheduler's heartbeat loop indefinitely.
    ///
    /// Every second, iterates over all workflows in topological order and
    /// fires any task whose `interval` has elapsed since its `last_run`.
    /// Tasks that have never run (`last_run: None`) fire on the first tick.
    ///
    /// This method blocks forever — callers typically spawn it on a
    /// dedicated tokio task and abort the handle to shut down.
    pub async fn run(&mut self) {
        let mut heartbeat = tokio::time::interval(Duration::from_secs(1));

        loop {
            heartbeat.tick().await;
            let now = Instant::now();

            for workflow in &mut self.workflows {
                for &idx in &workflow.execution_order {
                    let state = &mut workflow.tasks[idx];
                    let due = match state.last_run {
                        None => true,
                        Some(last) => now.duration_since(last) >= state.interval,
                    };

                    if due {
                        println!(
                            "[pneuma] workflow={} task={} status=fired",
                            workflow.name, workflow.task_names[idx]
                        );
                        state.last_run = Some(now);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ScheduledWorkflow;

    fn parse_workflow(yaml: &str) -> ScheduledWorkflow {
        serde_yml::from_str(yaml).unwrap()
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

        let mut scheduler = Pyra::new(vec![workflow]).unwrap();

        // Run the scheduler for a short window and verify it doesn't panic.
        // The stub just prints, so we confirm construction + one tick works.
        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tokio::time::sleep(Duration::from_millis(1500)).await;
        handle.abort();
    }
}
