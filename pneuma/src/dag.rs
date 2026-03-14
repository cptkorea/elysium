use std::collections::{HashMap, HashSet, VecDeque};

use crate::schema::WorkflowDef;
use crate::Error;

#[derive(Debug)]
pub struct Dag {
    /// task_name -> list of tasks it depends on
    dependencies: HashMap<String, Vec<String>>,
    /// task_name -> list of tasks that depend on it
    dependents: HashMap<String, Vec<String>>,
    nodes: HashSet<String>,
}

impl Dag {
    pub fn from_workflow(workflow: &WorkflowDef) -> Result<Self, Error> {
        let mut dependencies: HashMap<String, Vec<String>> = HashMap::new();
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        let mut nodes = HashSet::new();
        let all_tasks: HashSet<&str> = workflow.all_task_names().into_iter().collect();

        let mut prev_stage_tasks: Vec<String> = Vec::new();

        for stage in &workflow.stages {
            let current_stage_tasks: Vec<String> = stage.tasks.keys().cloned().collect();

            for task_name in &current_stage_tasks {
                nodes.insert(task_name.clone());
                let deps = dependencies.entry(task_name.clone()).or_default();

                // Implicit dependency: every task in this stage depends on all
                // tasks from the previous stage.
                for prev_task in &prev_stage_tasks {
                    deps.push(prev_task.clone());
                    dependents
                        .entry(prev_task.clone())
                        .or_default()
                        .push(task_name.clone());
                }

                // Explicit intra-stage depends_on
                if let Some(task_def) = stage.tasks.get(task_name) {
                    for dep in &task_def.depends_on {
                        if !all_tasks.contains(dep.as_str()) {
                            return Err(Error::UnknownTask(dep.clone()));
                        }
                        deps.push(dep.clone());
                        dependents
                            .entry(dep.clone())
                            .or_default()
                            .push(task_name.clone());
                    }
                }
            }

            prev_stage_tasks = current_stage_tasks;
        }

        Ok(Self {
            dependencies,
            dependents,
            nodes,
        })
    }

    /// Returns tasks in a valid execution order using Kahn's algorithm.
    pub fn execution_order(&self) -> Result<Vec<String>, Error> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        for node in &self.nodes {
            in_degree.insert(node.as_str(), 0);
        }

        for (task, deps) in &self.dependencies {
            in_degree.insert(task.as_str(), deps.len());
        }

        let mut queue: VecDeque<String> = VecDeque::new();
        for (node, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(node.to_string());
            }
        }

        let mut order = Vec::new();
        while let Some(task) = queue.pop_front() {
            order.push(task.clone());

            if let Some(downstream) = self.dependents.get(&task) {
                for dep in downstream {
                    if let Some(deg) = in_degree.get_mut(dep.as_str()) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            let remaining: Vec<String> = self
                .nodes
                .iter()
                .filter(|n| !order.contains(n))
                .cloned()
                .collect();
            return Err(Error::CycleDetected(remaining.join(", ")));
        }

        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::WorkflowDef;

    fn parse_workflow(yaml: &str) -> WorkflowDef {
        serde_yml::from_str(yaml).unwrap()
    }

    #[test]
    fn stage_ordering() {
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

        let dag = Dag::from_workflow(&workflow).unwrap();
        let order = dag.execution_order().unwrap();

        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();

        // a and b must come before c
        assert!(pos("a") < pos("c"));
        assert!(pos("b") < pos("c"));
        // c must come before d
        assert!(pos("c") < pos("d"));
    }

    #[test]
    fn intra_stage_depends_on() {
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

        let dag = Dag::from_workflow(&workflow).unwrap();
        let order = dag.execution_order().unwrap();

        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();
        assert!(pos("a") < pos("b"));
    }

    #[test]
    fn unknown_task_reference() {
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

        let result = Dag::from_workflow(&workflow);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }
}
