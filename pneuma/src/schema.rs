use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::Error;

#[derive(Debug, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    pub stages: Vec<StageDef>,
}

#[derive(Debug, Deserialize)]
pub struct StageDef {
    pub name: String,
    pub tasks: HashMap<String, TaskDef>,
}

#[derive(Debug, Deserialize)]
pub struct TaskDef {
    pub interval_secs: u64,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

impl WorkflowDef {
    pub fn from_yaml(path: &Path) -> Result<Self, Error> {
        let contents = std::fs::read_to_string(path)?;
        let workflow: WorkflowDef = serde_yml::from_str(&contents)?;
        Ok(workflow)
    }

    pub fn all_task_names(&self) -> Vec<&str> {
        self.stages
            .iter()
            .flat_map(|stage| stage.tasks.keys().map(|k| k.as_str()))
            .collect()
    }
}

pub fn load_workflows(dir: &Path) -> Result<Vec<WorkflowDef>, Error> {
    let mut workflows = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            workflows.push(WorkflowDef::from_yaml(&path)?);
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
        let workflow: WorkflowDef = serde_yml::from_str(yaml).unwrap();

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
        let workflow: WorkflowDef = serde_yml::from_str(yaml).unwrap();
        let task_b = &workflow.stages[0].tasks["b"];
        assert_eq!(task_b.depends_on, vec!["a"]);
    }
}
