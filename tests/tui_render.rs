use std::collections::BTreeMap;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use hugdocker::config::{AppConfig, ThemeName};
use hugdocker::domain::{Container, ContainerState, DockerSnapshot, OperationAction, SortMode};
use hugdocker::resources::{ResourcePanelData, ResourceRow, ResourceTrend};
use hugdocker::tui::{
    ContextMenuItem, ContextMenuState, DashboardState, MouseAction, TuiPanel, apply_enter_key,
    apply_mouse_action, begin_execution_prompt, execution_plan_if_confirmed,
    mark_resource_refresh_pending, mouse_action_for_event, push_execution_token, render_dashboard,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn sample_snapshot() -> DockerSnapshot {
    DockerSnapshot::from_containers(
        vec![
            Container {
                id: "web".into(),
                name: "web_1".into(),
                image: "example/web:latest".into(),
                state: ContainerState::Running,
                status: "Up 10 minutes".into(),
                compose_project: Some("mingli".into()),
                stack_namespace: None,
                labels: BTreeMap::new(),
                networks: vec!["mingli_default".into()],
                volumes: vec!["mingli_data".into()],
                ports: vec!["127.0.0.1:8080->80/tcp".into()],
            },
            Container {
                id: "worker".into(),
                name: "worker_1".into(),
                image: "example/worker:latest".into(),
                state: ContainerState::Unhealthy,
                status: "Up 1 minute (unhealthy)".into(),
                compose_project: Some("mingli".into()),
                stack_namespace: None,
                labels: BTreeMap::new(),
                networks: vec!["mingli_default".into()],
                volumes: vec![],
                ports: vec![],
            },
        ],
        vec!["mingli_default".into()],
        vec!["mingli_data".into()],
        vec!["example/web:latest".into(), "example/worker:latest".into()],
        &AppConfig::default(),
    )
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    }
}

fn two_project_snapshot() -> DockerSnapshot {
    DockerSnapshot::from_containers(
        vec![
            Container {
                id: "alpha-web".into(),
                name: "alpha_web_1".into(),
                image: "example/alpha:latest".into(),
                state: ContainerState::Running,
                status: "Up 3 minutes".into(),
                compose_project: Some("alpha".into()),
                stack_namespace: None,
                labels: BTreeMap::new(),
                networks: vec!["alpha_default".into()],
                volumes: vec![],
                ports: vec![],
            },
            Container {
                id: "mingli-web".into(),
                name: "mingli_web_1".into(),
                image: "example/mingli:latest".into(),
                state: ContainerState::Running,
                status: "Up 5 minutes".into(),
                compose_project: Some("mingli".into()),
                stack_namespace: None,
                labels: BTreeMap::new(),
                networks: vec!["mingli_default".into()],
                volumes: vec![],
                ports: vec![],
            },
        ],
        vec!["alpha_default".into(), "mingli_default".into()],
        vec![],
        vec![
            "example/alpha:latest".into(),
            "example/mingli:latest".into(),
        ],
        &AppConfig::default(),
    )
}

fn rendered_text_has_fg(buffer: &ratatui::buffer::Buffer, needle: &str, fg: Color) -> bool {
    let area = *buffer.area();
    for y in area.y..area.y + area.height {
        let mut line = String::new();
        for x in area.x..area.x + area.width {
            line.push_str(buffer.cell((x, y)).expect("cell").symbol());
        }
        if let Some(start) = line.find(needle) {
            return needle.chars().enumerate().all(|(offset, _)| {
                buffer
                    .cell((area.x + start as u16 + offset as u16, y))
                    .expect("cell")
                    .fg
                    == fg
            });
        }
    }
    false
}

fn rendered_text_has_bg(buffer: &ratatui::buffer::Buffer, needle: &str, bg: Color) -> bool {
    let area = *buffer.area();
    for y in area.y..area.y + area.height {
        let mut line = String::new();
        for x in area.x..area.x + area.width {
            line.push_str(buffer.cell((x, y)).expect("cell").symbol());
        }
        if let Some(start) = line.find(needle) {
            return needle.chars().enumerate().all(|(offset, _)| {
                buffer
                    .cell((area.x + start as u16 + offset as u16, y))
                    .expect("cell")
                    .bg
                    == bg
            });
        }
    }
    false
}

#[test]
fn dashboard_renders_command_center_metrics_and_project_table() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Detail;

    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    let rendered = format!("{buffer:?}");

    assert!(rendered.contains("HUGDOCKER COMMAND CENTER"));
    assert!(rendered.contains("Projects"));
    assert!(rendered.contains("Risk"));
    assert!(rendered.contains("mingli"));
    assert!(rendered.contains("Ops Deck"));
    assert!(rendered.contains("unhealthy"));
}

#[test]
fn cockpit_theme_renders_ops_cockpit_language() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.theme = ThemeName::Cockpit;

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("OPS COCKPIT"));
    assert!(rendered.contains("mode:all"));
    assert!(rendered.contains("selected:0"));
    assert!(rendered.contains("KPI"));
}

#[test]
fn ops_inbox_panel_renders_prioritized_next_actions() {
    let mut state = DashboardState::from_snapshot(sample_snapshot(), SortMode::Severity);
    state.panel = TuiPanel::Inbox;
    state.resource_data = Some(ResourcePanelData::sampled(
        "mingli",
        1_000,
        vec![ResourceRow::ok(
            "web", "web_1", "UP", 91.0, 900, 1_000, 10, 20, 30, 40,
        )],
    ));

    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Ops Inbox"));
    assert!(rendered.contains("Critical"));
    assert!(rendered.contains("Resource Pressure"));
    assert!(rendered.contains("Next Action"));
    assert!(rendered.contains("mingli"));
    assert!(rendered.contains("hugdocker rescue"));
}

#[test]
fn signal_theme_changes_header_accent_color() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.theme = ThemeName::Signal;

    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();

    assert!(rendered_text_has_fg(buffer, "HUGDOCKER", Color::Green));
}

#[test]
fn compact_terminal_renders_resize_hint_instead_of_full_dashboard() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    let backend = TestBackend::new(70, 16);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Terminal too small"));
    assert!(rendered.contains("90x22"));
    assert!(!rendered.contains("Ops Deck"));
}

#[test]
fn compact_terminal_ignores_mouse_project_selection() {
    let click = mouse(MouseEventKind::Down(MouseButton::Left), 5, 10);

    assert_eq!(mouse_action_for_event(click, (70, 16), 1, None), None);
}

#[test]
fn mouse_click_selects_project_and_switches_safe_panel() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(&mut state, MouseAction::ProjectRowClick { row: 0 });
    assert_eq!(
        state.current_project().map(|project| project.name.as_str()),
        Some("mingli")
    );
    assert!(state.selected.contains("mingli"));

    apply_mouse_action(&mut state, MouseAction::PanelClick { slot: 1 });
    assert_eq!(state.panel, TuiPanel::Doctor);

    apply_mouse_action(&mut state, MouseAction::ScrollDown);
    assert_eq!(state.table_state.selected(), Some(0));
}

#[test]
fn mouse_selected_row_keeps_color_after_cursor_moves() {
    let snapshot = two_project_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::NameAsc);

    apply_mouse_action(&mut state, MouseAction::ProjectRowClick { row: 0 });
    state.next();

    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();

    assert!(state.selected.contains("alpha"));
    assert_eq!(
        state.current_project().map(|project| project.name.as_str()),
        Some("mingli")
    );
    assert!(rendered_text_has_fg(buffer, "alpha", Color::Yellow));
}

#[test]
fn right_click_menu_renders_project_management_actions() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(
        &mut state,
        MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 12,
        },
    );

    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    let rendered = format!("{buffer:?}");

    assert!(rendered.contains("Manage mingli"));
    assert!(rendered.contains("Inspect"));
    assert!(rendered.contains("Doctor"));
    assert!(rendered.contains("Start"));
    assert!(rendered.contains("Stop"));
    assert!(rendered.contains("Restart"));
    assert!(rendered.contains("Rescue"));
    assert!(rendered.contains("Logs"));
    assert!(rendered.contains("Resources"));
    assert!(rendered.contains("Exec"));
    assert!(rendered.contains("Remove"));
    assert!(rendered.contains("Purge"));
}

#[test]
fn context_menu_highlights_current_menu_item() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(
        &mut state,
        MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 12,
        },
    );
    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuHover {
            item: ContextMenuItem::Restart,
        },
    );

    let backend = TestBackend::new(110, 32);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();

    assert!(rendered_text_has_bg(buffer, "Restart", Color::Cyan));
}

#[test]
fn context_menu_action_opens_plan_and_closes_menu() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(
        &mut state,
        MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 12,
        },
    );
    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuClick {
            item: ContextMenuItem::Rescue,
        },
    );

    assert_eq!(state.panel, TuiPanel::Plan(OperationAction::Rescue));
    assert!(state.context_menu.is_none());
    assert!(state.selected.contains("mingli"));
}

#[test]
fn context_menu_logs_and_resources_open_observability_panels() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(
        &mut state,
        MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 12,
        },
    );
    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuClick {
            item: ContextMenuItem::Logs,
        },
    );
    assert_eq!(state.panel, TuiPanel::Logs);

    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuClick {
            item: ContextMenuItem::Resources,
        },
    );
    assert_eq!(state.panel, TuiPanel::Resources);

    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuClick {
            item: ContextMenuItem::Exec,
        },
    );
    assert_eq!(state.panel, TuiPanel::Resources);
    assert_eq!(state.exec_container_index, Some(0));
    assert!(state.status.contains("Exec"));
}

#[test]
fn exec_picker_renders_active_container_choices() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuClick {
            item: ContextMenuItem::Exec,
        },
    );

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Select container shell"));
    assert!(rendered.contains("web_1"));
    assert!(rendered.contains("worker_1"));
    assert!(rendered.contains("sh -> bash -> ash"));
}

#[test]
fn command_palette_renders_fast_ops_commands() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::CommandPalette;

    let backend = TestBackend::new(130, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Command Palette"));
    assert!(rendered.contains("hugdocker update mingli --dry-run"));
    assert!(rendered.contains("hugdocker compose mingli pull --dry-run"));
}

#[test]
fn logs_panel_renders_log_lens_controls() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Logs;
    state.filter = "web".into();

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Ops Deck / Logs"));
    assert!(rendered.contains("Log Lens"));
    assert!(rendered.contains("Keyword Filter"));
    assert!(rendered.contains("Selected Container"));
    assert!(rendered.contains("ERROR"));
    assert!(rendered.contains("WARN"));
    assert!(rendered.contains("n/p switch container"));
    assert!(rendered.contains("hugdocker logs web"));
}

#[test]
fn resources_panel_renders_resource_monitor_summary() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Resources;

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Ops Deck / Resources"));
    assert!(rendered.contains("Resource Monitor"));
    assert!(rendered.contains("CPU"));
    assert!(rendered.contains("MEM"));
    assert!(rendered.contains("NET"));
    assert!(rendered.contains("IO"));
    assert!(rendered.contains("hotspot"));
}

#[test]
fn resources_panel_renders_project_trend_snapshot() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Resources;
    state.resource_data = Some(ResourcePanelData::sampled(
        "mingli",
        2_000,
        vec![ResourceRow::ok(
            "web", "web_1", "UP", 18.5, 180, 1_000, 140, 260, 360, 520,
        )],
    ));
    state.resource_trend = Some(ResourceTrend {
        cpu_delta_percent: 8.5,
        memory_delta_bytes: 80,
        network_rx_delta_bytes: 40,
        network_tx_delta_bytes: 60,
        block_read_delta_bytes: 60,
        block_write_delta_bytes: 120,
    });

    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("trend +8.5%"));
}

#[test]
fn resources_panel_renders_loading_state() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Resources;
    state.resource_data = Some(ResourcePanelData::loading("mingli"));

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Resource Monitor"));
    assert!(rendered.contains("sampling"));
    assert!(rendered.contains("mingli"));
}

#[test]
fn resource_refresh_keeps_previous_sample_for_same_project() {
    let current = ResourcePanelData::sampled(
        "mingli",
        1_000,
        vec![ResourceRow::ok(
            "web", "web_1", "UP", 12.5, 512, 1_024, 100, 200, 300, 400,
        )],
    );

    let refreshed = mark_resource_refresh_pending(Some(current), "mingli").expect("data");

    assert!(!refreshed.loading);
    assert!(refreshed.stale);
    assert_eq!(refreshed.rows[0].container_name, "web_1");
}

#[test]
fn resource_refresh_uses_loading_state_when_project_changes() {
    let current = ResourcePanelData::sampled(
        "mingli",
        1_000,
        vec![ResourceRow::ok(
            "web", "web_1", "UP", 12.5, 512, 1_024, 100, 200, 300, 400,
        )],
    );

    let refreshed = mark_resource_refresh_pending(Some(current), "other").expect("data");

    assert!(refreshed.loading);
    assert_eq!(refreshed.project, "other");
    assert!(refreshed.rows.is_empty());
}

#[test]
fn resources_panel_renders_sampled_summary_table_errors_and_stale_marker() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Resources;
    state.resource_data = Some(
        ResourcePanelData::sampled(
            "mingli",
            1_000,
            vec![
                ResourceRow::ok("web", "web_1", "UP", 12.5, 512, 1_024, 100, 200, 300, 400),
                ResourceRow::error("worker", "worker_1", "UNHL", "Docker returned 500"),
            ],
        )
        .with_stale(true),
    );

    let backend = TestBackend::new(140, 38);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Resource Monitor"));
    assert!(rendered.contains("refreshing"));
    assert!(rendered.contains("CPU"));
    assert!(rendered.contains("12.5%"));
    assert!(rendered.contains("MEM"));
    assert!(rendered.contains("50.0%"));
    assert!(rendered.contains("NET rx/tx"));
    assert!(rendered.contains("IO r/w"));
    assert!(rendered.contains("Container"));
    assert!(rendered.contains("web_1"));
    assert!(rendered.contains("worker_1"));
    assert!(rendered.contains("Docker returned 500"));
}

#[test]
fn resources_panel_renders_empty_active_container_state() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Resources;
    state.resource_data = Some(ResourcePanelData::sampled("mingli", 1_000, Vec::new()));

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let rendered = format!("{:?}", terminal.backend().buffer());

    assert!(rendered.contains("Resource Monitor"));
    assert!(rendered.contains("No active containers"));
}

#[test]
fn context_menu_restart_opens_only_restart_plan() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(
        &mut state,
        MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 12,
        },
    );
    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuClick {
            item: ContextMenuItem::Restart,
        },
    );
    begin_execution_prompt(&mut state);

    let plan = execution_plan_if_confirmed(&state)
        .expect("plan")
        .expect("confirmed plan");

    assert_eq!(state.panel, TuiPanel::Plan(OperationAction::Restart));
    assert_eq!(plan.action, OperationAction::Restart);
    assert_eq!(plan.confirmation_token, None);
}

#[test]
fn enter_on_logs_panel_does_not_switch_to_stop_plan() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Logs;

    apply_enter_key(&mut state);

    assert_eq!(state.panel, TuiPanel::Logs);
    assert!(state.execution_prompt.is_none());
    assert!(state.status.contains("只执行当前 Plan"));
}

#[test]
fn ops_deck_mouse_click_does_not_choose_destructive_plan() {
    let right_panel_click = mouse(MouseEventKind::Down(MouseButton::Left), 80, 20);

    assert_eq!(
        mouse_action_for_event(right_panel_click, (110, 32), 1, None),
        None
    );
}

#[test]
fn panel_click_never_selects_destructive_plan() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    for slot in 0..8 {
        apply_mouse_action(&mut state, MouseAction::PanelClick { slot });
        assert_ne!(state.panel, TuiPanel::Plan(OperationAction::Remove));
        assert_ne!(state.panel, TuiPanel::Plan(OperationAction::Purge));
    }
}

#[test]
fn rescue_plan_renders_recovery_playbook() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(
        &mut state,
        MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 12,
        },
    );
    apply_mouse_action(
        &mut state,
        MouseAction::ContextMenuClick {
            item: ContextMenuItem::Rescue,
        },
    );

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    let rendered = format!("{buffer:?}");

    assert!(rendered.contains("Recovery Playbook"));
    assert!(rendered.contains("异常信号"));
    assert!(rendered.contains("unhealthy"));
    assert!(rendered.contains("hugdocker rescue mingli --dry-run"));
}

#[test]
fn execution_prompt_renders_second_enter_confirmation() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Plan(OperationAction::Rescue);

    begin_execution_prompt(&mut state);

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    let rendered = format!("{buffer:?}");

    assert!(rendered.contains("Execute Rescue"));
    assert!(rendered.contains("Enter again to execute"));
    assert!(rendered.contains("Esc to cancel"));
}

#[test]
fn dangerous_execution_requires_typed_token() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Plan(OperationAction::Purge);

    begin_execution_prompt(&mut state);
    assert!(execution_plan_if_confirmed(&state).expect("plan").is_none());

    for ch in "DELETE-mingli".chars() {
        push_execution_token(&mut state, ch);
    }

    let plan = execution_plan_if_confirmed(&state)
        .expect("plan")
        .expect("confirmed plan");
    assert_eq!(plan.action, OperationAction::Purge);
    assert_eq!(plan.confirmation_token, Some("DELETE-mingli".into()));
}

#[test]
fn destructive_plan_renders_safety_rail() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);
    state.panel = TuiPanel::Plan(OperationAction::Purge);

    let backend = TestBackend::new(120, 36);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|frame| render_dashboard(frame, &mut state))
        .expect("draw");

    let buffer = terminal.backend().buffer();
    let rendered = format!("{buffer:?}");

    assert!(rendered.contains("Safety Rail"));
    assert!(rendered.contains("DELETE-mingli"));
    assert!(rendered.contains("mouse cannot execute destructive actions"));
}

#[test]
fn context_menu_can_close_without_action() {
    let snapshot = sample_snapshot();
    let mut state = DashboardState::from_snapshot(snapshot, SortMode::Severity);

    apply_mouse_action(
        &mut state,
        MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 12,
        },
    );
    assert!(state.context_menu.is_some());

    apply_mouse_action(&mut state, MouseAction::CloseContextMenu);
    assert!(state.context_menu.is_none());
}

#[test]
fn mouse_event_maps_right_click_and_menu_item_clicks() {
    let right_click = mouse(MouseEventKind::Down(MouseButton::Right), 5, 10);
    assert_eq!(
        mouse_action_for_event(right_click, (110, 32), 1, None),
        Some(MouseAction::OpenContextMenu {
            row: 0,
            x: 5,
            y: 10,
        })
    );

    let menu = ContextMenuState {
        project: "mingli".into(),
        row: 0,
        x: 5,
        y: 10,
        selected_index: 0,
    };
    let rescue_click = mouse(MouseEventKind::Down(MouseButton::Left), 6, 16);
    assert_eq!(
        mouse_action_for_event(rescue_click, (110, 32), 1, Some(&menu)),
        Some(MouseAction::ContextMenuClick {
            item: ContextMenuItem::Rescue,
        })
    );

    let exec_click = mouse(MouseEventKind::Down(MouseButton::Left), 6, 19);
    assert_eq!(
        mouse_action_for_event(exec_click, (110, 32), 1, Some(&menu)),
        Some(MouseAction::ContextMenuClick {
            item: ContextMenuItem::Exec,
        })
    );

    let outside_click = mouse(MouseEventKind::Down(MouseButton::Left), 100, 30);
    assert_eq!(
        mouse_action_for_event(outside_click, (110, 32), 1, Some(&menu)),
        Some(MouseAction::CloseContextMenu)
    );
}

#[test]
fn mouse_event_maps_context_menu_hover() {
    let menu = ContextMenuState {
        project: "mingli".into(),
        row: 0,
        x: 5,
        y: 10,
        selected_index: 0,
    };
    let restart_hover = mouse(MouseEventKind::Moved, 6, 15);

    assert_eq!(
        mouse_action_for_event(restart_hover, (110, 32), 1, Some(&menu)),
        Some(MouseAction::ContextMenuHover {
            item: ContextMenuItem::Restart,
        })
    );
}

#[test]
fn mouse_event_ignores_clicks_outside_main_area() {
    let header_click = mouse(MouseEventKind::Down(MouseButton::Left), 20, 1);
    assert_eq!(
        mouse_action_for_event(header_click, (110, 32), 1, None),
        None
    );

    let footer_click = mouse(MouseEventKind::Down(MouseButton::Left), 20, 31);
    assert_eq!(
        mouse_action_for_event(footer_click, (110, 32), 1, None),
        None
    );
}
