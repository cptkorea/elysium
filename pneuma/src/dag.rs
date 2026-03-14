use std::collections::{HashMap, HashSet, VecDeque};

use crate::schema::ScheduledWorkflow;
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
    pub fn from_workflow(workflow: &ScheduledWorkflow) -> Result<Self, Error> {
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

    /// Tasks with no dependencies (entry points of the DAG).
    pub fn roots(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|n| self.dependencies.get(n.as_str()).map_or(true, |d| d.is_empty()))
            .map(|n| n.as_str())
            .collect()
    }

    /// Tasks with no dependents (terminal tasks of the DAG).
    pub fn leaves(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|n| self.dependents.get(n.as_str()).map_or(true, |d| d.is_empty()))
            .map(|n| n.as_str())
            .collect()
    }

    /// Direct upstream dependencies of a task.
    pub fn dependencies_of(&self, task: &str) -> &[String] {
        self.dependencies.get(task).map_or(&[], |v| v.as_slice())
    }

    /// Direct downstream dependents of a task.
    pub fn dependents_of(&self, task: &str) -> &[String] {
        self.dependents.get(task).map_or(&[], |v| v.as_slice())
    }

    /// Total number of tasks in the DAG.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
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
    use crate::schema::ScheduledWorkflow;

    fn parse_workflow(yaml: &str) -> ScheduledWorkflow {
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
    fn roots_and_leaves() {
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

        let mut roots = dag.roots();
        roots.sort();
        assert_eq!(roots, vec!["a", "b"]);

        assert_eq!(dag.leaves(), vec!["d"]);
    }

    #[test]
    fn dependencies_and_dependents() {
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

        let dag = Dag::from_workflow(&workflow).unwrap();

        assert_eq!(dag.dependencies_of("a"), &[] as &[String]);
        assert_eq!(dag.dependencies_of("b"), &["a".to_string()]);
        assert_eq!(dag.dependencies_of("c"), &["a".to_string()]);

        let mut a_dependents = dag.dependents_of("a").to_vec();
        a_dependents.sort();
        assert_eq!(a_dependents, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn len_and_is_empty() {
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

        let dag = Dag::from_workflow(&workflow).unwrap();
        assert_eq!(dag.len(), 2);
        assert!(!dag.is_empty());
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
