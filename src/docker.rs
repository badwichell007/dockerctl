use std::collections::{BTreeMap, HashMap};
use std::process::Command;

use bollard::Docker;
use bollard::query_parameters::{
    EventsOptionsBuilder, ListContainersOptionsBuilder, ListImagesOptionsBuilder,
    LogsOptionsBuilder, PruneImagesOptionsBuilder, RemoveContainerOptionsBuilder,
    RestartContainerOptionsBuilder, StartContainerOptions, StatsOptionsBuilder,
    StopContainerOptionsBuilder,
};
use futures_util::TryStreamExt;
use futures_util::future::join_all;

use crate::audit::{AuditEntry, now_unix_millis, write_audit};
use crate::config::AppConfig;
use crate::domain::{Container, ContainerState, DockerSnapshot, OperationAction, Project};
use crate::ops::{OperationFailure, OperationPlan, OperationResult};
use crate::resources::{ResourcePanelData, ResourceRow, cpu_percent};
use crate::telemetry::{TimelineEvent, write_timeline};
use crate::{AppResult, msg};

#[derive(Clone)]
pub struct DockerClient {
    docker: Docker,
    config: AppConfig,
}

impl DockerClient {
    pub fn connect(config: AppConfig) -> AppResult<Self> {
        let docker = Docker::connect_with_socket_defaults()?;
        Ok(Self { docker, config })
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub async fn ping(&self) -> AppResult<()> {
        self.docker.ping().await?;
        Ok(())
    }

    pub async fn snapshot(&self) -> AppResult<DockerSnapshot> {
        let containers = self.list_containers().await?;
        let networks = self.list_network_names().await.unwrap_or_default();
        let volumes = self.list_volume_names().await.unwrap_or_default();
        let images = self.list_image_names().await.unwrap_or_default();
        Ok(DockerSnapshot::from_containers(
            containers,
            networks,
            volumes,
            images,
            &self.config,
        ))
    }

    async fn list_containers(&self) -> AppResult<Vec<Container>> {
        let options = ListContainersOptionsBuilder::default().all(true).build();
        let rows = self.docker.list_containers(Some(options)).await?;
        let resource_futures = rows
            .iter()
            .map(|row| {
                let id = row.id.clone().unwrap_or_default();
                async move {
                    self.inspect_container_resources(&id)
                        .await
                        .unwrap_or_default()
                }
            })
            .collect::<Vec<_>>();
        let resources = join_all(resource_futures).await;
        let mut containers = Vec::with_capacity(rows.len());
        for (row, (networks, volumes)) in rows.into_iter().zip(resources) {
            let labels = row
                .labels
                .unwrap_or_default()
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            let name = row
                .names
                .unwrap_or_default()
                .into_iter()
                .next()
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string();
            let status = row.status.unwrap_or_default();
            let state_text = row.state.map(|state| state.to_string()).unwrap_or_default();
            let compose_project = labels
                .get("com.docker.compose.project")
                .filter(|value| !value.is_empty())
                .cloned();
            let stack_namespace = labels
                .get("com.docker.stack.namespace")
                .filter(|value| !value.is_empty())
                .cloned();
            let ports = row
                .ports
                .unwrap_or_default()
                .into_iter()
                .filter_map(|port| {
                    let private = port.private_port;
                    let typ = port
                        .typ
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "tcp".to_string());
                    match (port.ip, port.public_port) {
                        (Some(ip), Some(public)) => Some(format!("{ip}:{public}->{private}/{typ}")),
                        (_, Some(public)) => Some(format!("{public}->{private}/{typ}")),
                        _ => None,
                    }
                })
                .collect();

            let id = row.id.unwrap_or_default();
            containers.push(Container {
                id,
                name,
                image: row.image.unwrap_or_default(),
                state: ContainerState::from_docker_state(&state_text, &status),
                status,
                compose_project,
                stack_namespace,
                labels,
                networks,
                volumes,
                ports,
            });
        }
        Ok(containers)
    }

    async fn inspect_container_resources(&self, id: &str) -> AppResult<(Vec<String>, Vec<String>)> {
        let detail = self.docker.inspect_container(id, None).await?;
        let mut networks = Vec::new();
        if let Some(settings) = detail.network_settings {
            if let Some(map) = settings.networks {
                networks.extend(map.keys().cloned());
            }
        }

        let mut volumes = Vec::new();
        if let Some(mounts) = detail.mounts {
            for mount in mounts {
                if mount.typ.map(|value| value.to_string()).as_deref() == Some("volume") {
                    if let Some(name) = mount.name {
                        volumes.push(name);
                    }
                }
            }
        }
        networks.sort();
        networks.dedup();
        volumes.sort();
        volumes.dedup();
        Ok((networks, volumes))
    }

    async fn list_network_names(&self) -> AppResult<Vec<String>> {
        let networks = self.docker.list_networks(None).await?;
        Ok(networks
            .into_iter()
            .filter_map(|network| network.name)
            .collect())
    }

    async fn list_volume_names(&self) -> AppResult<Vec<String>> {
        let volumes = self
            .docker
            .list_volumes(None::<bollard::query_parameters::ListVolumesOptions>)
            .await?;
        Ok(volumes
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|volume| volume.name)
            .collect())
    }

    async fn list_image_names(&self) -> AppResult<Vec<String>> {
        let images = self
            .docker
            .list_images(Some(ListImagesOptionsBuilder::default().all(true).build()))
            .await?;
        let mut names = Vec::new();
        for image in images {
            names.extend(image.repo_tags);
        }
        Ok(names)
    }

    pub async fn execute_plan(
        &self,
        plan: &OperationPlan,
        dry_run: bool,
    ) -> AppResult<OperationResult> {
        if dry_run {
            return Ok(OperationResult {
                action: plan.action,
                success: Vec::new(),
                failed: Vec::new(),
            });
        }

        let started = now_unix_millis();
        let mut result = OperationResult {
            action: plan.action,
            success: Vec::new(),
            failed: Vec::new(),
        };

        match plan.action {
            OperationAction::Start => {
                for id in &plan.containers {
                    collect_result(
                        id,
                        self.docker
                            .start_container(id, None::<StartContainerOptions>)
                            .await,
                        &mut result,
                    );
                }
            }
            OperationAction::Stop => {
                for id in &plan.containers {
                    collect_result(
                        id,
                        self.docker
                            .stop_container(
                                id,
                                Some(StopContainerOptionsBuilder::default().t(10).build()),
                            )
                            .await,
                        &mut result,
                    );
                }
            }
            OperationAction::Restart | OperationAction::Rescue => {
                for id in &plan.containers {
                    collect_result(
                        id,
                        self.docker
                            .restart_container(
                                id,
                                Some(RestartContainerOptionsBuilder::default().t(10).build()),
                            )
                            .await,
                        &mut result,
                    );
                }
            }
            OperationAction::Remove | OperationAction::Purge => {
                for id in &plan.containers {
                    collect_result(
                        id,
                        self.docker
                            .remove_container(
                                id,
                                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
                            )
                            .await,
                        &mut result,
                    );
                }
                if plan.action == OperationAction::Purge {
                    for volume in &plan.volumes {
                        collect_result(
                            volume,
                            self.docker
                                .remove_volume(
                                    volume,
                                    None::<bollard::query_parameters::RemoveVolumeOptions>,
                                )
                                .await,
                            &mut result,
                        );
                    }
                    for image in &plan.images {
                        collect_result(
                            image,
                            self.docker
                                .remove_image(
                                    image,
                                    None::<bollard::query_parameters::RemoveImageOptions>,
                                    None,
                                )
                                .await
                                .map(|_| ()),
                            &mut result,
                        );
                    }
                }
            }
            OperationAction::Prune => {
                match self.docker.prune_containers(None).await {
                    Ok(response) => {
                        for id in response.containers_deleted.unwrap_or_default() {
                            result.success.push(format!("container:{id}"));
                        }
                    }
                    Err(err) => result.failed.push(OperationFailure {
                        target: "containers".to_string(),
                        message: err.to_string(),
                    }),
                }
                match self.docker.prune_networks(None).await {
                    Ok(response) => {
                        for id in response.networks_deleted.unwrap_or_default() {
                            result.success.push(format!("network:{id}"));
                        }
                    }
                    Err(err) => result.failed.push(OperationFailure {
                        target: "networks".to_string(),
                        message: err.to_string(),
                    }),
                }
                let mut filters = HashMap::new();
                filters.insert("dangling", vec!["true"]);
                match self
                    .docker
                    .prune_images(Some(
                        PruneImagesOptionsBuilder::default()
                            .filters(&filters)
                            .build(),
                    ))
                    .await
                {
                    Ok(response) => {
                        for item in response.images_deleted.unwrap_or_default() {
                            if let Some(deleted) = item.deleted.or(item.untagged) {
                                result.success.push(format!("image:{deleted}"));
                            }
                        }
                    }
                    Err(err) => result.failed.push(OperationFailure {
                        target: "dangling-images".to_string(),
                        message: err.to_string(),
                    }),
                }
            }
        }

        let status = if result.failed.is_empty() {
            "ok"
        } else {
            "error"
        };
        let message = if result.failed.is_empty() {
            String::new()
        } else {
            format!("{} 个目标失败", result.failed.len())
        };
        let _ = write_audit(&AuditEntry {
            ts: started,
            action: plan.action,
            targets: plan.projects.clone(),
            dry_run,
            status: status.to_string(),
            duration_ms: now_unix_millis().saturating_sub(started),
            message,
        });
        Ok(result)
    }

    pub async fn container_logs(&self, container: &str, tail: usize) -> AppResult<Vec<String>> {
        let tail_text = tail.to_string();
        let mut stream = self.docker.logs(
            container,
            Some(
                LogsOptionsBuilder::default()
                    .stdout(true)
                    .stderr(true)
                    .tail(&tail_text)
                    .timestamps(false)
                    .follow(false)
                    .build(),
            ),
        );
        let mut lines = Vec::new();
        while let Some(chunk) = stream.try_next().await? {
            lines.push(chunk.to_string());
        }
        Ok(lines)
    }

    pub async fn container_stats_once(&self, container: &str) -> AppResult<ContainerStats> {
        let mut stream = self.docker.stats(
            container,
            Some(StatsOptionsBuilder::default().stream(false).build()),
        );
        let Some(stats) = stream.try_next().await? else {
            return msg("Docker 未返回 stats。");
        };
        let memory_usage = stats
            .memory_stats
            .as_ref()
            .and_then(|memory| memory.usage)
            .unwrap_or(0);
        let memory_limit = stats
            .memory_stats
            .as_ref()
            .and_then(|memory| memory.limit)
            .unwrap_or(0);
        let cpu_total = stats
            .cpu_stats
            .as_ref()
            .and_then(|cpu| cpu.cpu_usage.as_ref())
            .and_then(|usage| usage.total_usage)
            .unwrap_or(0);
        let (network_rx_bytes, network_tx_bytes) = stats
            .networks
            .as_ref()
            .map(|networks| {
                networks
                    .values()
                    .fold((0, 0), |(rx_total, tx_total), network| {
                        (
                            rx_total + network.rx_bytes.unwrap_or(0),
                            tx_total + network.tx_bytes.unwrap_or(0),
                        )
                    })
            })
            .unwrap_or((0, 0));
        let (block_read_bytes, block_write_bytes) = stats
            .blkio_stats
            .as_ref()
            .and_then(|blkio| blkio.io_service_bytes_recursive.as_ref())
            .map(|entries| {
                entries
                    .iter()
                    .fold((0, 0), |(read_total, write_total), entry| {
                        match entry.op.as_deref().map(str::to_ascii_lowercase).as_deref() {
                            Some("read") => (read_total + entry.value.unwrap_or(0), write_total),
                            Some("write") => (read_total, write_total + entry.value.unwrap_or(0)),
                            _ => (read_total, write_total),
                        }
                    })
            })
            .unwrap_or((0, 0));
        Ok(ContainerStats {
            name: stats.name.unwrap_or_else(|| container.to_string()),
            memory_usage,
            memory_limit,
            cpu_total,
            cpu_percent: stats_cpu_percent(stats.cpu_stats.as_ref(), stats.precpu_stats.as_ref()),
            network_rx_bytes,
            network_tx_bytes,
            block_read_bytes,
            block_write_bytes,
        })
    }

    pub async fn project_resources_once(&self, project: &Project) -> ResourcePanelData {
        let targets = project
            .containers
            .iter()
            .filter(|container| container.state.is_active())
            .cloned()
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return ResourcePanelData::sampled(project.name.clone(), now_unix_millis(), Vec::new());
        }
        let rows = join_all(targets.iter().map(|container| async move {
            match self.container_stats_once(&container.id).await {
                Ok(stats) => ResourceRow::ok(
                    container.id.clone(),
                    container.name.clone(),
                    container.state.state_code(),
                    stats.cpu_percent,
                    stats.memory_usage,
                    stats.memory_limit,
                    stats.network_rx_bytes,
                    stats.network_tx_bytes,
                    stats.block_read_bytes,
                    stats.block_write_bytes,
                ),
                Err(err) => ResourceRow::error(
                    container.id.clone(),
                    container.name.clone(),
                    container.state.state_code(),
                    err.to_string(),
                ),
            }
        }))
        .await;
        ResourcePanelData::sampled(project.name.clone(), now_unix_millis(), rows)
    }

    pub async fn watch_timeline(&self) -> AppResult<()> {
        let mut filters = std::collections::HashMap::new();
        filters.insert("type", vec!["container", "image", "network", "volume"]);
        let mut stream = self.docker.events(Some(
            EventsOptionsBuilder::default().filters(&filters).build(),
        ));
        while let Some(event) = stream.try_next().await? {
            let actor_id = event
                .actor
                .as_ref()
                .and_then(|actor| actor.id.clone())
                .unwrap_or_default();
            let actor_name = event
                .actor
                .as_ref()
                .and_then(|actor| actor.attributes.as_ref())
                .and_then(|attrs| attrs.get("name").cloned())
                .unwrap_or_else(|| actor_id.clone());
            let action = event.action.unwrap_or_default();
            let source = event.typ.map(|value| value.to_string()).unwrap_or_default();
            let ts = event
                .time_nano
                .map(|value| (value / 1_000_000).max(0) as u128)
                .or_else(|| event.time.map(|value| (value * 1_000).max(0) as u128))
                .unwrap_or_else(now_unix_millis);
            let timeline = TimelineEvent {
                ts,
                source,
                action: action.clone(),
                actor: actor_name.clone(),
                message: format!("{actor_name} {action}"),
            };
            write_timeline(&timeline)?;
            println!("{}", serde_json::to_string(&timeline)?);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContainerStats {
    pub name: String,
    pub memory_usage: u64,
    pub memory_limit: u64,
    pub cpu_total: u64,
    pub cpu_percent: f64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub block_read_bytes: u64,
    pub block_write_bytes: u64,
}

fn stats_cpu_percent(
    current: Option<&bollard::models::ContainerCpuStats>,
    previous: Option<&bollard::models::ContainerCpuStats>,
) -> f64 {
    let Some(current) = current else {
        return 0.0;
    };
    let Some(previous) = previous else {
        return 0.0;
    };
    let current_total = current
        .cpu_usage
        .as_ref()
        .and_then(|usage| usage.total_usage)
        .unwrap_or(0);
    let previous_total = previous
        .cpu_usage
        .as_ref()
        .and_then(|usage| usage.total_usage)
        .unwrap_or(0);
    cpu_percent(
        current_total,
        previous_total,
        current.system_cpu_usage.unwrap_or(0),
        previous.system_cpu_usage.unwrap_or(0),
        current.online_cpus.unwrap_or(0),
    )
}

fn collect_result<T>(
    target: &str,
    operation: Result<T, bollard::errors::Error>,
    result: &mut OperationResult,
) {
    match operation {
        Ok(_) => result.success.push(target.to_string()),
        Err(err) => result.failed.push(OperationFailure {
            target: target.to_string(),
            message: err.to_string(),
        }),
    }
}

pub fn docker_cli_available() -> bool {
    Command::new("docker")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
