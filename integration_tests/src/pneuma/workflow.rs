#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::Duration;

    use pneuma::dag::Dag;
    use pneuma::schema::WorkflowDef;
    use pneuma::scheduler::Scheduler;

    const ETL_YAML: &str = r#"
name: etl
stages:
  - name: extract
    tasks:
      ingest_users:
        interval_secs: 1
      ingest_orders:
        interval_secs: 1

  - name: transform
    tasks:
      normalize:
        interval_secs: 1

  - name: load
    tasks:
      write_to_db:
        interval_secs: 1
"#;

    #[test]
    fn load_workflow_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("etl.yaml");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(ETL_YAML.as_bytes()).unwrap();

        let workflow = WorkflowDef::from_yaml(&path).unwrap();
        assert_eq!(workflow.name, "etl");
        assert_eq!(workflow.stages.len(), 3);
    }

    #[test]
    fn dag_respects_stage_order() {
        let workflow: WorkflowDef = serde_yml::from_str(ETL_YAML).unwrap();
        let dag = Dag::from_workflow(&workflow).unwrap();
        let order = dag.execution_order().unwrap();

        let pos = |name: &str| order.iter().position(|n| n == name).unwrap();

        // extract tasks must precede transform
        assert!(pos("ingest_users") < pos("normalize"));
        assert!(pos("ingest_orders") < pos("normalize"));
        // transform must precede load
        assert!(pos("normalize") < pos("write_to_db"));
    }

    #[tokio::test]
    async fn scheduler_runs_without_panic() {
        let workflow: WorkflowDef = serde_yml::from_str(ETL_YAML).unwrap();
        let mut scheduler = Scheduler::new(vec![workflow]).unwrap();

        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tokio::time::sleep(Duration::from_millis(2500)).await;
        handle.abort();
    }
}
