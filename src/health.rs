use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::domain::{ContainerState, DockerSnapshot, HealthStatus, ProjectHealth};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
    pub docker_cli: bool,
    pub docker_daemon: bool,
    pub projects: Vec<ProjectHealth>,
    pub findings: Vec<String>,
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

fn is_default_network(network: &str) -> bool {
    matches!(network, "bridge" | "host" | "none")
}
