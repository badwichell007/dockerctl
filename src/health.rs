use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::domain::{ContainerState, DockerSnapshot, HealthStatus, ProjectHealth};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub docker_cli: bool,
    pub docker_daemon: bool,
    pub projects: Vec<ProjectHealth>,
    pub findings: Vec<String>,
    pub fingerprints: Vec<ProjectFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFingerprint {
    pub project: String,
    pub risk_score: u16,
    pub signals: Vec<String>,
    pub suggested_command: String,
}

pub fn analyze_snapshot(snapshot: &DockerSnapshot) -> Vec<ProjectHealth> {
    snapshot
        .projects
        .iter()
        .map(|project| {
            let mut findings = Vec::new();
            if project.unhealthy > 0 {
                findings.push(format!("{} 个 unhealthy 容器", project.unhealthy));
            }
            if project.restarting > 0 {
                findings.push(format!("{} 个 restarting 容器", project.restarting));
            }
            if project.paused > 0 {
                findings.push(format!("{} 个 paused 容器", project.paused));
            }
            if has_duplicate_ports(&project.ports) {
                findings.push("检测到项目内端口重复映射".to_string());
            }
            for port in project
                .ports
                .iter()
                .filter(|port| port.starts_with("0.0.0.0:"))
            {
                findings.push(format!("公网监听端口暴露: {port}"));
            }
            for container in &project.containers {
                if container.state == ContainerState::Restarting {
                    findings.push(format!("restart loop: {}", container.name));
                }
                for volume in &container.volumes {
                    if looks_anonymous_volume(volume) {
                        findings.push(format!("疑似匿名卷: {volume}"));
                    }
                }
            }
            if project.kind == crate::domain::ProjectKind::Standalone
                && project.networks.iter().any(|network| network != "bridge")
            {
                findings.push("standalone 项目使用自定义网络，删除前确认是否共享。".to_string());
            }

            let status = if project.unhealthy > 0 || project.restarting > 0 {
                HealthStatus::Critical
            } else if project.paused > 0
                || has_duplicate_ports(&project.ports)
                || findings.iter().any(|finding| finding.contains("公网监听"))
            {
                HealthStatus::Warning
            } else {
                HealthStatus::Healthy
            };

            ProjectHealth {
                project: project.name.clone(),
                status,
                findings,
            }
        })
        .collect()
}

pub fn global_findings(snapshot: &DockerSnapshot) -> Vec<String> {
    let mut findings = Vec::new();
    let mut ports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut used_networks = BTreeSet::new();
    let mut used_volumes = BTreeSet::new();
    for project in &snapshot.projects {
        used_networks.extend(project.networks.iter().cloned());
        used_volumes.extend(project.volumes.iter().cloned());
        if project.images.len() >= 5 {
            findings.push(format!(
                "项目 {} 引用 {} 个镜像，疑似镜像膨胀。",
                project.name,
                project.images.len()
            ));
        }
        for port in &project.ports {
            if let Some(host_port) = host_port_key(port) {
                ports
                    .entry(host_port)
                    .or_default()
                    .insert(project.name.clone());
            }
        }
    }
    for (port, projects) in ports {
        if projects.len() > 1 {
            findings.push(format!(
                "端口 {} 同时出现在多个项目: {}",
                port,
                projects.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    let orphan_networks = snapshot
        .networks
        .iter()
        .filter(|network| !is_default_network(network) && !used_networks.contains(*network))
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    if !orphan_networks.is_empty() {
        findings.push(format!("疑似孤儿网络: {}", orphan_networks.join(", ")));
    }
    let orphan_volumes = snapshot
        .volumes
        .iter()
        .filter(|volume| !used_volumes.contains(*volume))
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    if !orphan_volumes.is_empty() {
        findings.push(format!("疑似孤儿卷: {}", orphan_volumes.join(", ")));
    }
    findings
}

pub fn project_fingerprints(snapshot: &DockerSnapshot) -> Vec<ProjectFingerprint> {
    let health = analyze_snapshot(snapshot);
    let port_conflicts = project_port_conflicts(snapshot);
    let shared_volumes = project_shared_volumes(snapshot);
    let mut fingerprints = snapshot
        .projects
        .iter()
        .map(|project| {
            let findings = health
                .iter()
                .find(|item| item.project == project.name)
                .map(|item| item.findings.clone())
                .unwrap_or_default();
            let mut signals = findings;
            if project.active() == 0 {
                signals.push("inactive".to_string());
            }
            if project
                .ports
                .iter()
                .any(|port| port.starts_with("0.0.0.0:"))
            {
                signals.push("public_bind".to_string());
            }
            if project.images.len() >= 5 {
                signals.push("image_bloat".to_string());
            }
            let stale_stopped = project
                .containers
                .iter()
                .filter(|container| looks_stale_stopped(container.state, &container.status))
                .count();
            if stale_stopped > 0 {
                signals.push(format!("stale_stopped:{stale_stopped}"));
            }
            if let Some(ports) = port_conflicts.get(&project.name) {
                for port in ports {
                    signals.push(format!("port_conflict:{port}"));
                }
            }
            if let Some(volumes) = shared_volumes.get(&project.name) {
                for volume in volumes {
                    signals.push(format!("shared_volume:{volume}"));
                }
            }
            signals.sort();
            signals.dedup();
            ProjectFingerprint {
                project: project.name.clone(),
                risk_score: project_risk_score(project, &signals),
                suggested_command: suggested_command(project, &signals),
                signals,
            }
        })
        .collect::<Vec<_>>();
    fingerprints.sort_by(|a, b| {
        b.risk_score
            .cmp(&a.risk_score)
            .then_with(|| a.project.cmp(&b.project))
    });
    fingerprints
}

fn project_risk_score(project: &crate::domain::Project, signals: &[String]) -> u16 {
    let mut score = 0;
    score += project.unhealthy as u16 * 40;
    score += project.restarting as u16 * 35;
    score += project.paused as u16 * 10;
    score += signals
        .iter()
        .filter(|signal| signal.contains("公网监听"))
        .count() as u16
        * 10;
    score += signals
        .iter()
        .filter(|signal| signal.contains("匿名卷"))
        .count() as u16
        * 8;
    score += signals
        .iter()
        .filter(|signal| *signal == "public_bind")
        .count() as u16
        * 8;
    score += signals
        .iter()
        .filter(|signal| *signal == "image_bloat")
        .count() as u16
        * 6;
    score += signals
        .iter()
        .filter(|signal| signal.starts_with("port_conflict:"))
        .count() as u16
        * 15;
    score += signals
        .iter()
        .filter(|signal| signal.starts_with("shared_volume:"))
        .count() as u16
        * 12;
    score += signals
        .iter()
        .filter(|signal| signal.starts_with("stale_stopped:"))
        .count() as u16
        * 10;
    score.min(100)
}

fn suggested_command(project: &crate::domain::Project, signals: &[String]) -> String {
    if project.unhealthy > 0 || project.restarting > 0 {
        return format!("hugdocker rescue {} --dry-run", project.name);
    }
    if signals
        .iter()
        .any(|signal| signal.starts_with("stale_stopped:"))
    {
        return "hugdocker safe-prune --dry-run".to_string();
    }
    if signals
        .iter()
        .any(|signal| signal.starts_with("shared_volume:"))
    {
        return format!("hugdocker inspect {}", project.name);
    }
    if signals.iter().any(|signal| signal == "inactive") {
        return format!("hugdocker plan remove {}", project.name);
    }
    format!("hugdocker inspect {}", project.name)
}

fn looks_anonymous_volume(volume: &str) -> bool {
    volume.len() >= 32 && volume.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn has_duplicate_ports(ports: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    for port in ports {
        if let Some(key) = host_port_key(port) {
            if !seen.insert(key) {
                return true;
            }
        }
    }
    false
}

fn host_port_key(port: &str) -> Option<String> {
    let (host, _) = port.split_once("->")?;
    Some(host.rsplit(':').next().unwrap_or(host).to_string())
}

fn looks_stale_stopped(state: ContainerState, status: &str) -> bool {
    if !matches!(
        state,
        ContainerState::Exited
            | ContainerState::Created
            | ContainerState::Dead
            | ContainerState::Unknown
    ) {
        return false;
    }
    let status = status.to_ascii_lowercase();
    [" day", " week", " month", " year"]
        .iter()
        .any(|needle| status.contains(needle))
}

fn project_port_conflicts(snapshot: &DockerSnapshot) -> BTreeMap<String, Vec<String>> {
    let mut by_port: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for project in &snapshot.projects {
        for port in &project.ports {
            if let Some(host_port) = host_port_key(port) {
                by_port
                    .entry(host_port)
                    .or_default()
                    .insert(project.name.clone());
            }
        }
    }

    let mut by_project: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (port, projects) in by_port {
        if projects.len() <= 1 {
            continue;
        }
        for project in projects {
            by_project.entry(project).or_default().push(port.clone());
        }
    }
    by_project
}

fn project_shared_volumes(snapshot: &DockerSnapshot) -> BTreeMap<String, Vec<String>> {
    let mut by_volume: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for project in &snapshot.projects {
        for volume in &project.volumes {
            by_volume
                .entry(volume.clone())
                .or_default()
                .insert(project.name.clone());
        }
    }

    let mut by_project: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (volume, projects) in by_volume {
        if projects.len() <= 1 {
            continue;
        }
        for project in projects {
            by_project.entry(project).or_default().push(volume.clone());
        }
    }
    by_project
}

fn is_default_network(network: &str) -> bool {
    matches!(network, "bridge" | "host" | "none")
}
