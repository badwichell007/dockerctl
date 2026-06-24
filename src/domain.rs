use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Compose,
    Stack,
    Standalone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerState {
    Running,
    Restarting,
    Paused,
    Exited,
    Created,
    Dead,
    Unhealthy,
    Unknown,
}

impl ContainerState {
    pub fn from_docker_state(state: &str, status: &str) -> Self {
        if status.to_ascii_lowercase().contains("(unhealthy)") {
            return Self::Unhealthy;
        }
        match state.to_ascii_lowercase().as_str() {
            "running" => Self::Running,
            "restarting" => Self::Restarting,
            "paused" => Self::Paused,
            "exited" => Self::Exited,
            "created" => Self::Created,
            "dead" => Self::Dead,
            _ => Self::Unknown,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Running | Self::Restarting | Self::Paused | Self::Unhealthy
        )
    }

    pub fn state_code(self) -> &'static str {
        match self {
            Self::Running => "UP",
            Self::Restarting => "RSTR",
            Self::Paused => "PAUS",
            Self::Unhealthy => "UNHL",
            Self::Exited | Self::Created | Self::Dead => "DOWN",
            Self::Unknown => "UNKN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    pub status: String,
    pub compose_project: Option<String>,
    pub stack_namespace: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
    pub ports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRef {
    pub id: String,
    pub name: String,
    pub kind: ResourceKind,
    pub project: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Container,
    Network,
    Volume,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub kind: ProjectKind,
    pub containers: Vec<Container>,
    pub running: usize,
    pub restarting: usize,
    pub paused: usize,
    pub unhealthy: usize,
    pub stopped: usize,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
    pub images: Vec<String>,
    pub ports: Vec<String>,
}

impl Project {
    pub fn active(&self) -> usize {
        self.running + self.restarting + self.paused + self.unhealthy
    }

    pub fn severity_rank(&self) -> usize {
        if self.unhealthy > 0 {
            return 6;
        }
        if self.restarting > 0 {
            return 5;
        }
        if self.paused > 0 {
            return 4;
        }
        if self.running > 0 {
            return 3;
        }
        if !self.containers.is_empty() {
            return 2;
        }
        1
    }

    pub fn state_code(&self) -> &'static str {
        if self.unhealthy > 0 {
            return "UNHL";
        }
        if self.restarting > 0 {
            return "RSTR";
        }
        if self.paused > 0 {
            return "PAUS";
        }
        if self.running > 0 {
            return "UP";
        }
        if !self.containers.is_empty() {
            return "DOWN";
        }
        "IDLE"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerSnapshot {
    pub projects: Vec<Project>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
    pub images: Vec<String>,
}

impl DockerSnapshot {
    pub fn empty() -> Self {
        Self {
            projects: Vec::new(),
            networks: Vec::new(),
            volumes: Vec::new(),
            images: Vec::new(),
        }
    }

    pub fn from_containers(
        containers: Vec<Container>,
        networks: Vec<String>,
        volumes: Vec<String>,
        images: Vec<String>,
        config: &AppConfig,
    ) -> Self {
        let mut grouped: BTreeMap<String, (ProjectKind, Vec<Container>)> = BTreeMap::new();
        for container in containers {
            let (kind, name) = classify_container(&container, config);
            grouped
                .entry(name)
                .and_modify(|entry| entry.1.push(container.clone()))
                .or_insert((kind, vec![container]));
        }

        let mut projects = Vec::with_capacity(grouped.len());
        for (name, (kind, containers)) in grouped {
            let mut project_networks = BTreeSet::new();
            let mut project_volumes = BTreeSet::new();
            let mut project_images = BTreeSet::new();
            let mut project_ports = BTreeSet::new();
            let mut running = 0;
            let mut restarting = 0;
            let mut paused = 0;
            let mut unhealthy = 0;
            let mut stopped = 0;

            for container in &containers {
                match container.state {
                    ContainerState::Running => running += 1,
                    ContainerState::Restarting => restarting += 1,
                    ContainerState::Paused => paused += 1,
                    ContainerState::Unhealthy => unhealthy += 1,
                    ContainerState::Exited
                    | ContainerState::Created
                    | ContainerState::Dead
                    | ContainerState::Unknown => stopped += 1,
                }
                project_images.insert(container.image.clone());
                for network in &container.networks {
                    project_networks.insert(network.clone());
                }
                for volume in &container.volumes {
                    project_volumes.insert(volume.clone());
                }
                for port in &container.ports {
                    project_ports.insert(port.clone());
                }
            }

            projects.push(Project {
                name,
                kind,
                containers,
                running,
                restarting,
                paused,
                unhealthy,
                stopped,
                networks: project_networks.into_iter().collect(),
                volumes: project_volumes.into_iter().collect(),
                images: project_images.into_iter().collect(),
                ports: project_ports.into_iter().collect(),
            });
        }

        projects.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            projects,
            networks: unique_sorted(networks),
            volumes: unique_sorted(volumes),
            images: unique_sorted(images),
        }
    }

    pub fn project(&self, name: &str) -> Option<&Project> {
        self.projects.iter().find(|project| project.name == name)
    }

    pub fn projects_sorted(&self, mode: SortMode) -> Vec<Project> {
        let mut projects = self.projects.clone();
        sort_projects(&mut projects, mode);
        projects
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    Severity,
    NameAsc,
    ActiveDesc,
}

pub fn sort_projects(projects: &mut [Project], mode: SortMode) {
    projects.sort_by(|a, b| match mode {
        SortMode::Severity => b
            .severity_rank()
            .cmp(&a.severity_rank())
            .then_with(|| b.active().cmp(&a.active()))
            .then_with(|| b.containers.len().cmp(&a.containers.len()))
            .then_with(|| a.name.cmp(&b.name)),
        SortMode::NameAsc => a.name.cmp(&b.name),
        SortMode::ActiveDesc => b
            .active()
            .cmp(&a.active())
            .then_with(|| b.severity_rank().cmp(&a.severity_rank()))
            .then_with(|| a.name.cmp(&b.name)),
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationAction {
    Start,
    Stop,
    Restart,
    Remove,
    Purge,
    Prune,
    Rescue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectHealth {
    pub project: String,
    pub status: HealthStatus,
    pub findings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Warning,
    Critical,
}

fn classify_container(container: &Container, config: &AppConfig) -> (ProjectKind, String) {
    if let Some(project) = container
        .compose_project
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return (ProjectKind::Compose, project.to_string());
    }
    if let Some(project) = container
        .labels
        .get("com.docker.compose.project")
        .filter(|value| !value.is_empty())
    {
        return (ProjectKind::Compose, project.clone());
    }
    if let Some(stack) = container
        .stack_namespace
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        return (ProjectKind::Stack, stack.to_string());
    }
    if let Some(stack) = container
        .labels
        .get("com.docker.stack.namespace")
        .filter(|value| !value.is_empty())
    {
        return (ProjectKind::Stack, stack.clone());
    }
    if let Some(service) = container
        .labels
        .get("com.docker.compose.service")
        .filter(|value| !value.is_empty())
    {
        if let Some(project) = infer_compose_project_from_name(&container.name, service) {
            return (ProjectKind::Compose, project);
        }
    }
    (
        ProjectKind::Standalone,
        standalone_group_name(container, config),
    )
}

fn infer_compose_project_from_name(name: &str, service: &str) -> Option<String> {
    for separator in ['-', '_'] {
        let marker = format!("{separator}{service}{separator}");
        let Some((project, index)) = name.rsplit_once(&marker) else {
            continue;
        };
        if !project.is_empty() && index.chars().all(|ch| ch.is_ascii_digit()) {
            return Some(project.to_string());
        }
    }
    None
}

pub fn standalone_group_name(container: &Container, config: &AppConfig) -> String {
    if let Some(group) = config.groups.exact.get(&container.name) {
        return group.clone();
    }
    for (prefix, group) in &config.groups.prefix {
        if container.name.starts_with(prefix) {
            return group.clone();
        }
    }
    for (image_prefix, group) in &config.groups.image_prefix {
        if container.image.starts_with(image_prefix) {
            return group.clone();
        }
    }
    container.name.clone()
}

pub fn labels_from_pairs(
    pairs: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, String> {
    pairs.into_iter().collect()
}

fn unique_sorted(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
