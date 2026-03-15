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
//!   produced by [`DirectedAcyclicGraph::execution_order`](crate::dag::DirectedAcyclicGraph::execution_order),
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
/// The `execution_order` vector lists task names in topological order —
/// stage boundaries are implicit (all tasks from stage N appear before
/// any task from stage N+1). The `tasks` map holds the per-task timing
/// state keyed by task name.
struct ResolvedWorkflow {
    name: String,
    execution_order: Vec<String>,
    tasks: HashMap<String, TaskState>,
}

impl Pyra {
    /// Creates a new scheduler from a list of workflow definitions.
    ///
    /// For each workflow, this:
    /// 1. Builds the dependency graph via [`DirectedAcyclicGraph::from_workflow`].
    /// 2. Computes a topological execution order via [`DirectedAcyclicGraph::execution_order`].
    /// 3. Extracts per-task intervals into [`TaskState`] with `last_run: None`.
    ///
    /// Returns an error if any workflow contains an unknown dependency
    /// reference or a cycle.
    pub fn new(workflows: Vec<ScheduledWorkflow>) -> Result<Self, Error> {
        let mut resolved = Vec::new();

        for workflow in &workflows {
            let dag = workflow.to_dag()?;
            let order = dag.execution_order()?;

            let mut tasks = HashMap::new();
            for stage in &workflow.stages {
                for (name, def) in &stage.tasks {
                    tasks.insert(
                        name.clone(),
                        TaskState {
                            interval: Duration::from_secs(def.interval_secs),
                            last_run: None,
                        },
                    );
                }
            }

            resolved.push(ResolvedWorkflow {
                name: workflow.name.clone(),
                execution_order: order,
                tasks,
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
                for task_name in &workflow.execution_order {
                    if let Some(state) = workflow.tasks.get_mut(task_name) {
                        let due = match state.last_run {
                            None => true,
                            Some(last) => now.duration_since(last) >= state.interval,
                        };

                        if due {
                            println!(
                                "[pneuma] workflow={} task={} status=fired",
                                workflow.name, task_name
                            );
                            state.last_run = Some(now);
                        }
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
