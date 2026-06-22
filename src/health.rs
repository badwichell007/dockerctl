use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::domain::{DockerSnapshot, HealthStatus, ProjectHealth};

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
            if has_duplicate_ports(&project.ports) {
                findings.push("检测到项目内端口重复映射".to_string());
            }

            let status = if project.unhealthy > 0 || project.restarting > 0 {
                HealthStatus::Critical
            } else if project.paused > 0 || has_duplicate_ports(&project.ports) {
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
    for project in &snapshot.projects {
        for port in &project.ports {
            if let Some(host_port) = host_port_key(port) {
                ports.entry(host_port).or_default().insert(project.name.clone());
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
    findings
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
