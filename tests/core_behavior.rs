use dockerctl::config::{AppConfig, ThemeName, parse_group_config, parse_theme};
use dockerctl::domain::{
    Container, ContainerState, DockerSnapshot, OperationAction, ProjectKind, SortMode,
};
use dockerctl::ops::OperationPlanner;

fn fixture_snapshot() -> DockerSnapshot {
    let mut config = AppConfig::default();
    config.groups.prefix.push(("redis-".into(), "cache".into()));

    DockerSnapshot::from_containers(
        vec![
            Container {
                id: "c1".into(),
                name: "web_1".into(),
                image: "example/web:latest".into(),
                state: ContainerState::Running,
                status: "Up 2 minutes".into(),
                compose_project: Some("shop".into()),
                stack_namespace: None,
                labels: [("com.docker.compose.project".into(), "shop".into())].into(),
                networks: vec!["shop_default".into()],
                volumes: vec!["shop_data".into()],
                ports: vec!["0.0.0.0:8080->80/tcp".into()],
            },
            Container {
                id: "c2".into(),
                name: "worker_1".into(),
                image: "example/worker:latest".into(),
                state: ContainerState::Unhealthy,
                status: "Up 1 minute (unhealthy)".into(),
                compose_project: Some("shop".into()),
                stack_namespace: None,
                labels: [("com.docker.compose.project".into(), "shop".into())].into(),
                networks: vec!["shop_default".into()],
                volumes: vec![],
                ports: vec![],
            },
            Container {
                id: "c3".into(),
                name: "redis-main".into(),
                image: "redis:7".into(),
                state: ContainerState::Exited,
                status: "Exited (0)".into(),
                compose_project: None,
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["bridge".into()],
                volumes: vec!["redis_data".into()],
                ports: vec![],
            },
        ],
        vec!["shop_default".into(), "bridge".into()],
        vec!["shop_data".into(), "redis_data".into()],
        vec![
            "example/web:latest".into(),
            "example/worker:latest".into(),
            "redis:7".into(),
        ],
        &config,
    )
}

#[test]
fn config_supports_exact_prefix_and_image_prefix_groups() {
    let config = parse_group_config(
        r#"
[group_exact]
"one-off" = "tools"

[group_prefix]
"redis-" = "cache"

[group_image_prefix]
"postgres:" = "database"
"#,
    );

    assert_eq!(
        config.exact.get("one-off").map(String::as_str),
        Some("tools")
    );
    assert_eq!(config.prefix, vec![("redis-".into(), "cache".into())]);
    assert_eq!(
        config.image_prefix,
        vec![("postgres:".into(), "database".into())]
    );
}

#[test]
fn config_supports_custom_profile_groups() {
    let config = toml::from_str::<AppConfig>(
        r#"
[profiles.groups]
"frontend" = ["web*", "nginx*"]
"data" = ["postgres*", "redis*"]
"#,
    )
    .expect("profile config");

    assert_eq!(
        config.profiles.groups.get("frontend"),
        Some(&vec!["web*".to_string(), "nginx*".to_string()])
    );
    assert_eq!(
        config.profiles.groups.get("data"),
        Some(&vec!["postgres*".to_string(), "redis*".to_string()])
    );
}

#[test]
fn snapshot_groups_compose_and_standalone_containers_in_one_pass() {
    let snapshot = fixture_snapshot();

    let shop = snapshot.project("shop").expect("shop project");
    assert_eq!(shop.kind, ProjectKind::Compose);
    assert_eq!(shop.containers.len(), 2);
    assert_eq!(shop.running, 1);
    assert_eq!(shop.unhealthy, 1);
    assert_eq!(shop.networks, vec!["shop_default"]);
    assert_eq!(shop.volumes, vec!["shop_data"]);

    let cache = snapshot.project("cache").expect("standalone group");
    assert_eq!(cache.kind, ProjectKind::Standalone);
    assert_eq!(cache.containers.len(), 1);
    assert_eq!(cache.stopped, 1);
}

#[test]
fn project_sorting_prioritizes_unhealthy_projects() {
    let snapshot = fixture_snapshot();
    let names: Vec<String> = snapshot
        .projects_sorted(SortMode::Severity)
        .into_iter()
        .map(|project| project.name)
        .collect();

    assert_eq!(names, vec!["shop", "cache"]);
}

#[test]
fn remove_plan_describes_resources_without_touching_docker() {
    let snapshot = fixture_snapshot();
    let plan = OperationPlanner::new(&snapshot)
        .plan(OperationAction::Remove, &["shop".into()])
        .expect("remove plan");

    assert_eq!(plan.action, OperationAction::Remove);
    assert_eq!(plan.projects, vec!["shop"]);
    assert_eq!(plan.containers, vec!["c1", "c2"]);
    assert_eq!(plan.networks, vec!["shop_default"]);
    assert!(plan.volumes.is_empty());
    assert!(plan.images.is_empty());
    assert!(plan.confirmation_token.is_some());
    assert!(plan.summary.contains("删除 2 个容器"));
}

#[test]
fn purge_plan_requires_stronger_confirmation_and_includes_images() {
    let snapshot = fixture_snapshot();
    let plan = OperationPlanner::new(&snapshot)
        .plan(OperationAction::Purge, &["shop".into()])
        .expect("purge plan");

    assert_eq!(plan.volumes, vec!["shop_data"]);
    assert_eq!(
        plan.images,
        vec!["example/web:latest", "example/worker:latest"]
    );
    assert_eq!(plan.confirmation_token.as_deref(), Some("DELETE-shop"));
}

#[test]
fn compose_project_can_be_inferred_from_labels_and_container_name() {
    let snapshot = DockerSnapshot::from_containers(
        vec![Container {
            id: "compose-1".into(),
            name: "store-api-1".into(),
            image: "example/api:latest".into(),
            state: ContainerState::Running,
            status: "Up 10 seconds".into(),
            compose_project: None,
            stack_namespace: None,
            labels: [
                ("com.docker.compose.service".into(), "api".into()),
                ("com.docker.compose.version".into(), "2.24.0".into()),
            ]
            .into(),
            networks: vec!["store_default".into()],
            volumes: vec![],
            ports: vec![],
        }],
        vec!["store_default".into()],
        vec![],
        vec!["example/api:latest".into()],
        &AppConfig::default(),
    );

    let store = snapshot.project("store").expect("inferred compose project");
    assert_eq!(store.kind, ProjectKind::Compose);
    assert_eq!(store.containers[0].name, "store-api-1");
}

#[test]
fn config_parses_named_tui_themes() {
    let config: AppConfig = toml::from_str(
        r#"
[tui]
theme = "signal"
"#,
    )
    .expect("config");

    assert_eq!(parse_theme(&config.tui.theme), ThemeName::Signal);
    assert_eq!(
        parse_theme(&AppConfig::default().tui.theme),
        ThemeName::Cockpit
    );
    assert_eq!(parse_theme("cockpit"), ThemeName::Cockpit);
    assert_eq!(parse_theme("unknown"), ThemeName::Industrial);
}

#[test]
fn prune_plan_documents_safe_scope_and_requires_token() {
    let snapshot = fixture_snapshot();
    let plan = OperationPlanner::new(&snapshot)
        .plan(OperationAction::Prune, &[])
        .expect("prune plan");

    assert_eq!(plan.action, OperationAction::Prune);
    assert_eq!(plan.confirmation_token.as_deref(), Some("PRUNE"));
    assert!(plan.summary.contains("stopped containers"));
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("volumes are excluded"))
    );
}
