use dockerctl::config::AppConfig;
use dockerctl::domain::{Container, ContainerState, DockerSnapshot, HealthStatus};
use dockerctl::health::{analyze_snapshot, global_findings};

#[test]
fn doctor_reports_ports_public_bind_restart_loop_and_anonymous_volume() {
    let snapshot = DockerSnapshot::from_containers(
        vec![
            Container {
                id: "web".into(),
                name: "web_1".into(),
                image: "example/web:latest".into(),
                state: ContainerState::Restarting,
                status: "Restarting".into(),
                compose_project: Some("shop".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["shop_default".into()],
                volumes: vec!["0123456789abcdef0123456789abcdef".into()],
                ports: vec!["0.0.0.0:8080->80/tcp".into()],
            },
            Container {
                id: "api".into(),
                name: "api_1".into(),
                image: "example/api:latest".into(),
                state: ContainerState::Running,
                status: "Up".into(),
                compose_project: Some("api".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["api_default".into()],
                volumes: vec![],
                ports: vec!["127.0.0.1:8080->80/tcp".into()],
            },
            Container {
                id: "fat".into(),
                name: "fat_1".into(),
                image: "example/fat-a:latest".into(),
                state: ContainerState::Running,
                status: "Up".into(),
                compose_project: Some("fat".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["fat_default".into()],
                volumes: vec![],
                ports: vec![],
            },
            Container {
                id: "fat2".into(),
                name: "fat_2".into(),
                image: "example/fat-b:latest".into(),
                state: ContainerState::Running,
                status: "Up".into(),
                compose_project: Some("fat".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["fat_default".into()],
                volumes: vec![],
                ports: vec![],
            },
            Container {
                id: "fat3".into(),
                name: "fat_3".into(),
                image: "example/fat-c:latest".into(),
                state: ContainerState::Running,
                status: "Up".into(),
                compose_project: Some("fat".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["fat_default".into()],
                volumes: vec![],
                ports: vec![],
            },
            Container {
                id: "fat4".into(),
                name: "fat_4".into(),
                image: "example/fat-d:latest".into(),
                state: ContainerState::Running,
                status: "Up".into(),
                compose_project: Some("fat".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["fat_default".into()],
                volumes: vec![],
                ports: vec![],
            },
            Container {
                id: "fat5".into(),
                name: "fat_5".into(),
                image: "example/fat-e:latest".into(),
                state: ContainerState::Running,
                status: "Up".into(),
                compose_project: Some("fat".into()),
                stack_namespace: None,
                labels: Default::default(),
                networks: vec!["fat_default".into()],
                volumes: vec![],
                ports: vec![],
            },
        ],
        vec![
            "shop_default".into(),
            "api_default".into(),
            "fat_default".into(),
            "orphan_net".into(),
        ],
        vec![
            "0123456789abcdef0123456789abcdef".into(),
            "orphan_volume".into(),
        ],
        vec![
            "example/web:latest".into(),
            "example/api:latest".into(),
            "example/fat-a:latest".into(),
            "example/fat-b:latest".into(),
            "example/fat-c:latest".into(),
            "example/fat-d:latest".into(),
            "example/fat-e:latest".into(),
        ],
        &AppConfig::default(),
    );

    let reports = analyze_snapshot(&snapshot);
    let shop = reports
        .iter()
        .find(|report| report.project == "shop")
        .expect("shop");

    assert_eq!(shop.status, HealthStatus::Critical);
    assert!(
        shop.findings
            .iter()
            .any(|finding| finding.contains("restart loop"))
    );
    assert!(
        shop.findings
            .iter()
            .any(|finding| finding.contains("公网监听"))
    );
    assert!(
        shop.findings
            .iter()
            .any(|finding| finding.contains("匿名卷"))
    );
    let global = global_findings(&snapshot);
    assert!(global.iter().any(|finding| finding.contains("端口 8080")));
    assert!(global.iter().any(|finding| finding.contains("镜像膨胀")));
    assert!(global.iter().any(|finding| finding.contains("孤儿网络")));
    assert!(global.iter().any(|finding| finding.contains("孤儿卷")));
}

#[test]
fn recipes_have_stable_scriptable_shape() {
    let recipes = dockerctl::recipes::builtin_recipes();

    assert!(recipes.iter().any(|recipe| recipe.name == "panic-stop"));
    assert!(
        recipes
            .iter()
            .any(|recipe| recipe.name == "rescue-unhealthy")
    );
    assert!(
        recipes
            .iter()
            .any(|recipe| recipe.name == "preflight-delete")
    );
    assert!(
        recipes
            .iter()
            .all(|recipe| recipe.command.starts_with("dockerctl "))
    );
}
