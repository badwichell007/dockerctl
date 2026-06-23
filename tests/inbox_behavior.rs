use dockerctl::config::AppConfig;
use dockerctl::domain::{Container, ContainerState, DockerSnapshot};
use dockerctl::inbox::{build_ops_inbox, InboxSeverity};
use dockerctl::resources::{ResourcePanelData, ResourceRow};

fn snapshot_with_risk_and_cleanup() -> DockerSnapshot {
    DockerSnapshot::from_containers(
        vec![
            Container {
                id: "web".into(),
                name: "web_1".into(),
                image: "example/web:latest".into(),
                state: ContainerState::Running,
                status: "Up 10 minutes".into(),
                compose_project: Some("shop".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["shop_default".into()],
                volumes: vec!["shop_data".into()],
                ports: vec!["127.0.0.1:8080->80/tcp".into()],
            },
            Container {
                id: "worker".into(),
                name: "worker_1".into(),
                image: "example/worker:latest".into(),
                state: ContainerState::Unhealthy,
                status: "Up 1 minute (unhealthy)".into(),
                compose_project: Some("shop".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["shop_default".into()],
                volumes: vec![],
                ports: vec![],
            },
            Container {
                id: "old-job".into(),
                name: "old-job".into(),
                image: "example/job:latest".into(),
                state: ContainerState::Exited,
                status: "Exited (0) 4 hours ago".into(),
                compose_project: None,
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["bridge".into()],
                volumes: vec![],
                ports: vec![],
            },
        ],
        vec!["shop_default".into(), "bridge".into()],
        vec!["shop_data".into()],
        vec![
            "example/web:latest".into(),
            "example/worker:latest".into(),
            "example/job:latest".into(),
        ],
        &AppConfig::default(),
    )
}

#[test]
fn ops_inbox_prioritizes_risk_pressure_cleanup_and_next_action() {
    let snapshot = snapshot_with_risk_and_cleanup();
    let resources = ResourcePanelData::sampled(
        "shop",
        1_000,
        vec![
            ResourceRow::ok(
                "web",
                "web_1",
                "UP",
                91.2,
                920 * 1024 * 1024,
                1024 * 1024 * 1024,
                100,
                200,
                300,
                400,
            ),
            ResourceRow::error("worker", "worker_1", "UNHL", "Docker returned 500"),
        ],
    );

    let inbox = build_ops_inbox(&snapshot, Some(&resources));

    assert_eq!(inbox.items[0].severity, InboxSeverity::Critical);
    assert!(inbox.items.iter().any(|item| item.category == "Critical"));
    assert!(inbox.items.iter().any(|item| item.category == "Resource Pressure"));
    assert!(inbox.items.iter().any(|item| item.category == "Cleanup"));
    assert!(inbox.items.iter().any(|item| item.category == "Next Action"));
    assert!(inbox
        .items
        .iter()
        .any(|item| item.command == "dockerctl rescue shop --dry-run"));
    assert!(inbox
        .items
        .iter()
        .any(|item| item.command == "dockerctl safe-prune --dry-run"));
}

#[test]
fn ops_inbox_shows_calm_state_when_no_action_is_needed() {
    let snapshot = DockerSnapshot::empty();

    let inbox = build_ops_inbox(&snapshot, None);

    assert_eq!(inbox.items.len(), 1);
    assert_eq!(inbox.items[0].severity, InboxSeverity::Info);
    assert!(inbox.items[0].title.contains("No urgent action"));
}
