use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourcePanelData {
    pub project: String,
    pub sampled_at_ms: u128,
    pub loading: bool,
    pub stale: bool,
    pub rows: Vec<ResourceRow>,
    pub summary: ResourceSummary,
}

impl ResourcePanelData {
    pub fn loading(project: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            sampled_at_ms: 0,
            loading: true,
            stale: false,
            rows: Vec::new(),
            summary: ResourceSummary::default(),
        }
    }

    pub fn sampled(
        project: impl Into<String>,
        sampled_at_ms: u128,
        rows: Vec<ResourceRow>,
    ) -> Self {
        let summary = ResourceSummary::from_rows(&rows);
        Self {
            project: project.into(),
            sampled_at_ms,
            loading: false,
            stale: false,
            rows,
            summary,
        }
    }

    pub fn with_stale(mut self, stale: bool) -> Self {
        self.stale = stale;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceTrend {
    pub cpu_delta_percent: f64,
    pub memory_delta_bytes: i128,
    pub network_rx_delta_bytes: i128,
    pub network_tx_delta_bytes: i128,
    pub block_read_delta_bytes: i128,
    pub block_write_delta_bytes: i128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceRow {
    pub container_id: String,
    pub container_name: String,
    pub state: String,
    pub cpu_percent: f64,
    pub memory_usage_bytes: u64,
    pub memory_limit_bytes: u64,
    pub memory_percent: f64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub block_read_bytes: u64,
    pub block_write_bytes: u64,
    pub error: Option<String>,
}

impl ResourceRow {
    #[allow(clippy::too_many_arguments)]
    pub fn ok(
        container_id: impl Into<String>,
        container_name: impl Into<String>,
        state: impl Into<String>,
        cpu_percent: f64,
        memory_usage_bytes: u64,
        memory_limit_bytes: u64,
        network_rx_bytes: u64,
        network_tx_bytes: u64,
        block_read_bytes: u64,
        block_write_bytes: u64,
    ) -> Self {
        Self {
            container_id: container_id.into(),
            container_name: container_name.into(),
            state: state.into(),
            cpu_percent,
            memory_usage_bytes,
            memory_limit_bytes,
            memory_percent: memory_percent(memory_usage_bytes, memory_limit_bytes),
            network_rx_bytes,
            network_tx_bytes,
            block_read_bytes,
            block_write_bytes,
            error: None,
        }
    }

    pub fn error(
        container_id: impl Into<String>,
        container_name: impl Into<String>,
        state: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            container_id: container_id.into(),
            container_name: container_name.into(),
            state: state.into(),
            cpu_percent: 0.0,
            memory_usage_bytes: 0,
            memory_limit_bytes: 0,
            memory_percent: 0.0,
            network_rx_bytes: 0,
            network_tx_bytes: 0,
            block_read_bytes: 0,
            block_write_bytes: 0,
            error: Some(error.into()),
        }
    }

    pub fn ok_row(&self) -> bool {
        self.error.is_none()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceSummary {
    pub containers: usize,
    pub error_count: usize,
    pub cpu_percent: f64,
    pub memory_usage_bytes: u64,
    pub memory_limit_bytes: u64,
    pub memory_percent: f64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub block_read_bytes: u64,
    pub block_write_bytes: u64,
}

impl ResourceSummary {
    pub fn from_rows(rows: &[ResourceRow]) -> Self {
        let mut summary = Self {
            containers: rows.len(),
            error_count: rows.iter().filter(|row| row.error.is_some()).count(),
            ..Self::default()
        };
        for row in rows.iter().filter(|row| row.ok_row()) {
            summary.cpu_percent += row.cpu_percent;
            summary.memory_usage_bytes += row.memory_usage_bytes;
            summary.memory_limit_bytes += row.memory_limit_bytes;
            summary.network_rx_bytes += row.network_rx_bytes;
            summary.network_tx_bytes += row.network_tx_bytes;
            summary.block_read_bytes += row.block_read_bytes;
            summary.block_write_bytes += row.block_write_bytes;
        }
        summary.memory_percent =
            memory_percent(summary.memory_usage_bytes, summary.memory_limit_bytes);
        summary
    }
}

pub fn sorted_resource_rows(rows: &[ResourceRow]) -> Vec<ResourceRow> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| {
        resource_pressure_rank(b)
            .cmp(&resource_pressure_rank(a))
            .then_with(|| {
                b.cpu_percent
                    .partial_cmp(&a.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                b.memory_percent
                    .partial_cmp(&a.memory_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.container_name.cmp(&b.container_name))
    });
    rows
}

pub fn resource_pressure_hint(rows: &[ResourceRow]) -> Option<String> {
    let row = sorted_resource_rows(rows)
        .into_iter()
        .find(|row| row.error.is_some() || row.cpu_percent >= 80.0 || row.memory_percent >= 85.0)?;
    if let Some(error) = row.error {
        return Some(format!("{} stats error: {error}", row.container_name));
    }
    if row.cpu_percent >= 80.0 {
        return Some(format!(
            "{} high CPU {:.1}%",
            row.container_name, row.cpu_percent
        ));
    }
    Some(format!(
        "{} high memory {:.1}%",
        row.container_name, row.memory_percent
    ))
}

pub fn resource_trend(
    previous: &ResourcePanelData,
    current: &ResourcePanelData,
) -> Option<ResourceTrend> {
    if previous.project != current.project || previous.loading || current.loading {
        return None;
    }
    Some(ResourceTrend {
        cpu_delta_percent: current.summary.cpu_percent - previous.summary.cpu_percent,
        memory_delta_bytes: current.summary.memory_usage_bytes as i128
            - previous.summary.memory_usage_bytes as i128,
        network_rx_delta_bytes: current.summary.network_rx_bytes as i128
            - previous.summary.network_rx_bytes as i128,
        network_tx_delta_bytes: current.summary.network_tx_bytes as i128
            - previous.summary.network_tx_bytes as i128,
        block_read_delta_bytes: current.summary.block_read_bytes as i128
            - previous.summary.block_read_bytes as i128,
        block_write_delta_bytes: current.summary.block_write_bytes as i128
            - previous.summary.block_write_bytes as i128,
    })
}

pub fn format_signed_bytes(bytes: i128) -> String {
    let sign = if bytes > 0 {
        "+"
    } else if bytes < 0 {
        "-"
    } else {
        ""
    };
    let value = bytes.unsigned_abs().min(u64::MAX as u128) as u64;
    format!("{sign}{}", format_bytes(value))
}

fn resource_pressure_rank(row: &ResourceRow) -> usize {
    if row.error.is_some() {
        return 4;
    }
    if row.cpu_percent >= 80.0 {
        return 3;
    }
    if row.memory_percent >= 85.0 {
        return 2;
    }
    if row.cpu_percent >= 50.0 || row.memory_percent >= 70.0 {
        return 1;
    }
    0
}

pub fn cpu_percent(
    cpu_total: u64,
    previous_cpu_total: u64,
    system_cpu_total: u64,
    previous_system_cpu_total: u64,
    online_cpus: u32,
) -> f64 {
    let cpu_delta = cpu_total.saturating_sub(previous_cpu_total);
    let system_delta = system_cpu_total.saturating_sub(previous_system_cpu_total);
    if cpu_delta == 0 || system_delta == 0 || online_cpus == 0 {
        return 0.0;
    }
    (cpu_delta as f64 / system_delta as f64) * online_cpus as f64 * 100.0
}

pub fn memory_percent(memory_usage_bytes: u64, memory_limit_bytes: u64) -> f64 {
    if memory_usage_bytes == 0 || memory_limit_bytes == 0 {
        return 0.0;
    }
    memory_usage_bytes as f64 / memory_limit_bytes as f64 * 100.0
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}
