use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::{DockerSnapshot, OperationAction, Project, ProjectKind};
use crate::{AppResult, msg};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationPlan {
    pub action: OperationAction,
    pub projects: Vec<String>,
    pub containers: Vec<String>,
    pub networks: Vec<String>,
    pub volumes: Vec<String>,
    pub images: Vec<String>,
    pub confirmation_token: Option<String>,
    pub summary: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    pub action: OperationAction,
    pub success: Vec<String>,
    pub failed: Vec<OperationFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationFailure {
    pub target: String,
    pub message: String,
}

pub struct OperationPlanner<'a> {
    snapshot: &'a DockerSnapshot,
}

impl<'a> OperationPlanner<'a> {
    pub fn new(snapshot: &'a DockerSnapshot) -> Self {
        Self { snapshot }
    }

    pub fn plan(
        &self,
        action: OperationAction,
        project_names: &[String],
    ) -> AppResult<OperationPlan> {
        if project_names.is_empty() && action != OperationAction::Prune {
            return msg("请至少选择一个项目。");
        }

        let projects = self.resolve_projects(project_names)?;
        let mut containers = BTreeSet::new();
        let mut networks = BTreeSet::new();
        let mut volumes = BTreeSet::new();
        let mut images = BTreeSet::new();
        let mut warnings = Vec::new();

        for project in &projects {
            collect_project_resources(
                project,
                action,
                &mut containers,
                &mut networks,
                &mut volumes,
                &mut images,
                &mut warnings,
            );
        }

        if action == OperationAction::Prune {
            warnings.push(
                "safe-prune executes stopped containers, unused networks, and dangling images only; volumes are excluded."
                    .to_string(),
            );
            warnings.push(
                "执行前请确认没有依赖 stopped containers 或 dangling images 的离线调试流程。"
                    .to_string(),
            );
        }

        let project_names = projects
            .iter()
            .map(|project| project.name.clone())
            .collect::<Vec<_>>();
        let containers = containers.into_iter().collect::<Vec<_>>();
        let networks = networks.into_iter().collect::<Vec<_>>();
        let volumes = volumes.into_iter().collect::<Vec<_>>();
        let images = images.into_iter().collect::<Vec<_>>();
        let confirmation_token = confirmation_token(action, &project_names);
        let summary = plan_summary(action, &containers, &networks, &volumes, &images);

        Ok(OperationPlan {
            action,
            projects: project_names,
            containers,
            networks,
            volumes,
            images,
            confirmation_token,
            summary,
            warnings,
        })
    }

    fn resolve_projects(&self, project_names: &[String]) -> AppResult<Vec<&'a Project>> {
        let mut projects = Vec::with_capacity(project_names.len());
        for name in project_names {
            let Some(project) = self.snapshot.project(name) else {
                return msg(format!("未找到项目: {name}"));
            };
            projects.push(project);
        }
        Ok(projects)
    }
}

fn collect_project_resources(
    project: &Project,
    action: OperationAction,
    containers: &mut BTreeSet<String>,
    networks: &mut BTreeSet<String>,
    volumes: &mut BTreeSet<String>,
    images: &mut BTreeSet<String>,
    warnings: &mut Vec<String>,
) {
    match action {
        OperationAction::Start => {
            for container in &project.containers {
                containers.insert(container.id.clone());
            }
        }
        OperationAction::Stop | OperationAction::Restart | OperationAction::Rescue => {
            for container in &project.containers {
                if container.state.is_active() || action == OperationAction::Restart {
                    containers.insert(container.id.clone());
                }
            }
            if action == OperationAction::Rescue {
                if project.unhealthy == 0 && project.restarting == 0 {
                    warnings.push(format!(
                        "项目 {} 没有明显异常，rescue 将按 restart 处理。",
                        project.name
                    ));
                } else {
                    let mut signals = Vec::new();
                    if project.unhealthy > 0 {
                        signals.push(format!("{} unhealthy", project.unhealthy));
                    }
                    if project.restarting > 0 {
                        signals.push(format!("{} restarting", project.restarting));
                    }
                    warnings.push(format!(
                        "项目 {} 异常信号: {}",
                        project.name,
                        signals.join(", ")
                    ));
                }
            }
        }
        OperationAction::Remove | OperationAction::Purge => {
            for container in &project.containers {
                containers.insert(container.id.clone());
            }
            if project.kind == ProjectKind::Standalone {
                warnings.push(format!(
                    "项目 {} 是 standalone 分组，网络默认视为共享资源，不会自动删除。",
                    project.name
                ));
            } else {
                for network in &project.networks {
                    networks.insert(network.clone());
                }
            }
            if action == OperationAction::Purge {
                for volume in &project.volumes {
                    volumes.insert(volume.clone());
                }
                for image in &project.images {
                    images.insert(image.clone());
                }
            }
        }
        OperationAction::Prune => {}
    }
}

fn confirmation_token(action: OperationAction, projects: &[String]) -> Option<String> {
    match action {
        OperationAction::Remove => Some(if projects.len() == 1 {
            format!("REMOVE-{}", projects[0])
        } else {
            format!("REMOVE-BATCH-{}", projects.len())
        }),
        OperationAction::Purge => Some(if projects.len() == 1 {
            format!("DELETE-{}", projects[0])
        } else {
            format!("DELETE-BATCH-{}", projects.len())
        }),
        OperationAction::Prune => Some("PRUNE".to_string()),
        _ => None,
    }
}

fn plan_summary(
    action: OperationAction,
    containers: &[String],
    networks: &[String],
    volumes: &[String],
    images: &[String],
) -> String {
    match action {
        OperationAction::Start => format!("将启动 {} 个容器。", containers.len()),
        OperationAction::Stop => format!("将停止 {} 个活动容器。", containers.len()),
        OperationAction::Restart => format!("将重启 {} 个容器。", containers.len()),
        OperationAction::Remove => format!(
            "将删除 {} 个容器、{} 个网络，保留卷和镜像。",
            containers.len(),
            networks.len()
        ),
        OperationAction::Purge => format!(
            "将完全删除 {} 个容器、{} 个网络、{} 个卷、{} 个镜像。",
            containers.len(),
            networks.len(),
            volumes.len(),
            images.len()
        ),
        OperationAction::Prune => {
            "Safe-prune will target stopped containers, unused networks, and dangling images; volumes are excluded.".to_string()
        }
        OperationAction::Rescue => format!("将对 {} 个异常/活动容器执行恢复重启。", containers.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_token_uses_stronger_delete_for_purge() {
        assert_eq!(
            confirmation_token(OperationAction::Purge, &["app".into()]),
            Some("DELETE-app".into())
        );
    }
}
