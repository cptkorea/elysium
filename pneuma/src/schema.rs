use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::Error;

/// Top-level definition for a scheduled workflow, deserialized from a single YAML file.
/// Workflows contain an ordered list of stages that define the execution DAG.
#[derive(Debug, Deserialize)]
pub struct ScheduledWorkflow {
    pub name: String,
    pub stages: Vec<WorkflowStage>,
}

/// A named group of tasks within a workflow. Stages execute sequentially:
/// all tasks in stage N must complete before any task in stage N+1 begins.
/// Tasks within the same stage run independently unless constrained by `depends_on`.
#[derive(Debug, Deserialize)]
pub struct WorkflowStage {
    pub name: String,
    pub tasks: HashMap<String, WorkflowTask>,
}

/// A single schedulable unit of work within a stage.
#[derive(Debug, Deserialize)]
pub struct WorkflowTask {
    /// How often this task should fire, in seconds.
    pub interval_secs: u64,
    /// Optional intra-stage dependency references. Each entry must name another
    /// task within the same workflow. Use this when two tasks share a stage but
    /// one must run before the other.
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl ScheduledWorkflow {
    /// Deserialize a [`ScheduledWorkflow`] from a YAML file on disk.
    pub fn from_yaml(path: &Path) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(path)?;
        let workflow: ScheduledWorkflow = serde_yml::from_str(&contents)?;
        Ok(workflow)
    }

    /// Returns the names of every task across all stages in this workflow.
    pub fn all_task_names(&self) -> Vec<&str> {
        self.stages
            .iter()
            .flat_map(|stage| stage.tasks.keys().map(|k| k.as_str()))
            .collect()
    }
}

/// Load all `*.yaml` files from a directory, deserializing each as a [`ScheduledWorkflow`].
pub fn load_workflows(dir: &Path) -> Result<Vec<ScheduledWorkflow>, Error> {
    let mut workflows = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            workflows.push(ScheduledWorkflow::from_yaml(&path)?);
        }
    }
    Ok(workflows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_workflow_yaml() {
        let yaml = r#"
name: etl
stages:
  - name: extract
    tasks:
      ingest_users:
        interval_secs: 30
      ingest_orders:
        interval_secs: 30

  - name: transform
    tasks:
      normalize:
        interval_secs: 60

  - name: load
    tasks:
      write_to_db:
        interval_secs: 120
"#;
        let workflow: ScheduledWorkflow = serde_yml::from_str(yaml).unwrap();

        assert_eq!(workflow.name, "etl");
        assert_eq!(workflow.stages.len(), 3);
        assert_eq!(workflow.stages[0].name, "extract");
        assert_eq!(workflow.stages[0].tasks.len(), 2);
        assert!(workflow.stages[0].tasks.contains_key("ingest_users"));
        assert!(workflow.stages[0].tasks.contains_key("ingest_orders"));
        assert_eq!(workflow.stages[1].tasks["normalize"].interval_secs, 60);
        assert_eq!(workflow.stages[2].tasks["write_to_db"].interval_secs, 120);
        assert_eq!(workflow.all_task_names().len(), 4);
    }

    #[test]
    fn parse_with_depends_on() {
        let yaml = r#"
name: pipeline
stages:
  - name: stage_one
    tasks:
      a:
        interval_secs: 10
      b:
        interval_secs: 10
        depends_on:
          - a
"#;
        let workflow: ScheduledWorkflow = serde_yml::from_str(yaml).unwrap();
        let task_b = &workflow.stages[0].tasks["b"];
        assert_eq!(task_b.depends_on, vec!["a"]);
    }
}
