use std::collections::HashMap;
use std::path::Path;

use elysium_common::dag::DirectedAcyclicGraph;
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

    /// Builds a [`DirectedAcyclicGraph`] from this workflow definition.
    ///
    /// Walks stages in order, creating implicit edges from every task in the
    /// current stage to every task in the previous stage. Then processes
    /// explicit `depends_on` entries for intra-stage ordering.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownTask`] if any `depends_on` entry references
    /// a task name that doesn't exist in the workflow.
    pub fn to_dag(&self) -> Result<DirectedAcyclicGraph, Error> {
        let mut names = Vec::new();
        let mut index_of: HashMap<String, usize> = HashMap::new();

        for stage in &self.stages {
            for task_name in stage.tasks.keys() {
                let idx = names.len();
                names.push(task_name.clone());
                index_of.insert(task_name.clone(), idx);
            }
        }

        let mut edges: Vec<(usize, usize)> = Vec::new();
        let mut prev_stage_indices: Vec<usize> = Vec::new();

        for stage in &self.stages {
            let current_stage_indices: Vec<usize> = stage
                .tasks
                .keys()
                .map(|name| index_of[name])
                .collect();

            for &task_idx in &current_stage_indices {
                for &prev_idx in &prev_stage_indices {
                    edges.push((prev_idx, task_idx));
                }

                let task_name = &names[task_idx];
                if let Some(task_def) = stage.tasks.get(task_name) {
                    for dep_name in &task_def.depends_on {
                        let dep_idx = index_of.get(dep_name.as_str()).ok_or_else(|| {
                            Error::UnknownTask(dep_name.clone())
                        })?;
                        edges.push((*dep_idx, task_idx));
                    }
                }
            }

            prev_stage_indices = current_stage_indices;
        }

        Ok(DirectedAcyclicGraph::new(names, edges))
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

    fn parse_workflow(yaml: &str) -> ScheduledWorkflow {
        serde_yml::from_str(yaml).unwrap()
    }

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
    fn dag_stage_ordering() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: first
    tasks:
      a:
        interval_secs: 10
      b:
        interval_secs: 10
  - name: second
    tasks:
      c:
        interval_secs: 10
  - name: third
    tasks:
      d:
        interval_secs: 10
"#,
        );

        let dag = workflow.to_dag().unwrap();
        let order = dag.execution_order().unwrap();

        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();

        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("c"));
        assert!(pos("c") < pos("d"));
    }

    #[test]
    fn dag_intra_stage_depends_on() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: stage_one
    tasks:
      a:
        interval_secs: 10
      b:
        interval_secs: 10
        depends_on:
          - a
"#,
        );

        let dag = workflow.to_dag().unwrap();
        let order = dag.execution_order().unwrap();

        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();
        assert!(pos("a") < pos("b"));
    }

    #[test]
    fn dag_roots_and_leaves() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: first
    tasks:
      a:
        interval_secs: 10
      b:
        interval_secs: 10
  - name: second
    tasks:
      c:
        interval_secs: 10
  - name: third
    tasks:
      d:
        interval_secs: 10
"#,
        );

        let dag = workflow.to_dag().unwrap();

        let mut roots = dag.roots();
        roots.sort();
        assert_eq!(roots, vec!["a", "b"]);

        assert_eq!(dag.leaves(), vec!["d"]);
    }

    #[test]
    fn dag_dependencies_and_dependents() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: first
    tasks:
      a:
        interval_secs: 10
  - name: second
    tasks:
      b:
        interval_secs: 10
      c:
        interval_secs: 10
"#,
        );

        let dag = workflow.to_dag().unwrap();

        let empty: Vec<&str> = vec![];
        assert_eq!(dag.dependencies_of("a"), empty);
        assert_eq!(dag.dependencies_of("b"), vec!["a"]);
        assert_eq!(dag.dependencies_of("c"), vec!["a"]);

        let mut a_dependents = dag.dependents_of("a");
        a_dependents.sort();
        assert_eq!(a_dependents, vec!["b", "c"]);
    }

    #[test]
    fn dag_len_and_is_empty() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: only
    tasks:
      a:
        interval_secs: 10
      b:
        interval_secs: 10
"#,
        );

        let dag = workflow.to_dag().unwrap();
        assert_eq!(dag.len(), 2);
        assert!(!dag.is_empty());
    }

    #[test]
    fn dag_unknown_task_reference() {
        let workflow = parse_workflow(
            r#"
name: test
stages:
  - name: stage_one
    tasks:
      a:
        interval_secs: 10
        depends_on:
          - nonexistent
"#,
        );

        let result = workflow.to_dag();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
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
