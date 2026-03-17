#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use pneuma::scheduler::Pyra;
    use pneuma::schema::ScheduledWorkflow;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/pneuma")
            .join(name)
    }

    #[test]
    fn load_workflow_from_file() {
        let workflow = ScheduledWorkflow::from_yaml(&fixture("etl.yaml")).unwrap();
        assert_eq!(workflow.name, "etl");
        assert_eq!(workflow.stages.len(), 3);
    }

    #[test]
    fn dag_respects_stage_order() {
        let workflow = ScheduledWorkflow::from_yaml(&fixture("etl.yaml")).unwrap();
        let dag = workflow.to_dag().unwrap();
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
        let workflow = ScheduledWorkflow::from_yaml(&fixture("etl.yaml")).unwrap();
        let scheduler = Pyra::new(vec![workflow]).unwrap();

        let handle = tokio::spawn(async move {
            scheduler.run().await;
        });

        tokio::time::sleep(Duration::from_millis(2500)).await;
        handle.abort();
    }
}
