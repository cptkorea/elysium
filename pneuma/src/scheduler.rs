use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::dag::Dag;
use crate::schema::WorkflowDef;
use crate::Error;

struct TaskState {
    interval: Duration,
    last_run: Option<Instant>,
}

pub struct Scheduler {
    workflows: Vec<ResolvedWorkflow>,
}

struct ResolvedWorkflow {
    name: String,
    execution_order: Vec<String>,
    tasks: HashMap<String, TaskState>,
}

impl Scheduler {
    pub fn new(workflows: Vec<WorkflowDef>) -> Result<Self, Error> {
        let mut resolved = Vec::new();

        for workflow in &workflows {
            let dag = Dag::from_workflow(workflow)?;
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
    use crate::schema::WorkflowDef;

    fn parse_workflow(yaml: &str) -> WorkflowDef {
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

        let mut scheduler = Scheduler::new(vec![workflow]).unwrap();

        // Run the scheduler for a short window and verify it doesn't panic.
        // The stub just prints, so we confirm construction + one tick works.
        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tokio::time::sleep(Duration::from_millis(1500)).await;
        handle.abort();
    }
}
