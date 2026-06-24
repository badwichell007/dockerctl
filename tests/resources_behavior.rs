use dockerctl::resources::{
    ResourcePanelData, ResourceRow, ResourceSummary, cpu_percent, format_bytes,
    format_signed_bytes, resource_pressure_hint, resource_trend, sorted_resource_rows,
};

#[test]
fn resource_math_formats_bytes_and_cpu_percent() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(1_536), "1.5 KiB");
    assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");

    let percent = cpu_percent(150, 100, 1_000, 500, 2);
    assert!((percent - 20.0).abs() < f64::EPSILON);

    assert_eq!(cpu_percent(100, 100, 1_000, 500, 2), 0.0);
    assert_eq!(cpu_percent(150, 100, 1_000, 1_000, 2), 0.0);
}

#[test]
fn resource_summary_aggregates_success_rows_and_errors() {
    let rows = vec![
        ResourceRow::ok("web", "web_1", "UP", 12.5, 512, 1_024, 100, 200, 300, 400),
        ResourceRow::ok(
            "worker", "worker_1", "UNHL", 7.5, 256, 1_024, 10, 20, 30, 40,
        ),
        ResourceRow::error("gone", "gone_1", "DOWN", "Docker returned 404"),
    ];

    let summary = ResourceSummary::from_rows(&rows);

    assert_eq!(summary.containers, 3);
    assert_eq!(summary.error_count, 1);
    assert!((summary.cpu_percent - 20.0).abs() < f64::EPSILON);
    assert_eq!(summary.memory_usage_bytes, 768);
    assert_eq!(summary.memory_limit_bytes, 2_048);
    assert!((summary.memory_percent - 37.5).abs() < f64::EPSILON);
    assert_eq!(summary.network_rx_bytes, 110);
    assert_eq!(summary.network_tx_bytes, 220);
    assert_eq!(summary.block_read_bytes, 330);
    assert_eq!(summary.block_write_bytes, 440);
}

#[test]
fn resource_panel_data_marks_loading_empty_and_stale_states() {
    let loading = ResourcePanelData::loading("mingli");
    assert!(loading.loading);
    assert_eq!(loading.project, "mingli");
    assert!(loading.rows.is_empty());

    let empty = ResourcePanelData::sampled("mingli", 1_000, Vec::new());
    assert!(!empty.loading);
    assert!(empty.rows.is_empty());
    assert_eq!(empty.summary.containers, 0);

    let stale = empty.with_stale(true);
    assert!(stale.stale);
}

#[test]
fn resource_rows_sort_pressure_first_and_emit_hotspot_hint() {
    let rows = vec![
        ResourceRow::ok("calm", "calm_1", "UP", 3.0, 10, 100, 0, 0, 0, 0),
        ResourceRow::ok("cpu", "cpu_1", "UP", 88.0, 10, 100, 0, 0, 0, 0),
        ResourceRow::error("err", "err_1", "ERR", "stats timeout"),
    ];

    let sorted = sorted_resource_rows(&rows);

    assert_eq!(sorted[0].container_name, "err_1");
    assert_eq!(sorted[1].container_name, "cpu_1");
    assert_eq!(
        resource_pressure_hint(&rows).as_deref(),
        Some("err_1 stats error: stats timeout")
    );
}

#[test]
fn resource_trend_compares_adjacent_project_samples() {
    let previous = ResourcePanelData::sampled(
        "mingli",
        1_000,
        vec![ResourceRow::ok(
            "web", "web_1", "UP", 10.0, 100, 1_000, 100, 200, 300, 400,
        )],
    );
    let current = ResourcePanelData::sampled(
        "mingli",
        2_000,
        vec![ResourceRow::ok(
            "web", "web_1", "UP", 18.5, 180, 1_000, 140, 260, 360, 520,
        )],
    );

    let trend = resource_trend(&previous, &current).expect("trend");

    assert!((trend.cpu_delta_percent - 8.5).abs() < f64::EPSILON);
    assert_eq!(trend.memory_delta_bytes, 80);
    assert_eq!(trend.network_rx_delta_bytes, 40);
    assert_eq!(trend.network_tx_delta_bytes, 60);
    assert_eq!(trend.block_read_delta_bytes, 60);
    assert_eq!(trend.block_write_delta_bytes, 120);
    assert_eq!(format_signed_bytes(trend.memory_delta_bytes), "+80 B");
}
