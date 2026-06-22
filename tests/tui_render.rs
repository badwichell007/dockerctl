use std::collections::BTreeMap;

use dockerctl::config::AppConfig;
use dockerctl::domain::{Container, ContainerState, DockerSnapshot, OperationAction, SortMode};
use dockerctl::tui::{
    apply_mouse_action, begin_execution_prompt, execution_plan_if_confirmed,
    mouse_action_for_event, push_execution_token, render_dashboard, ContextMenuItem,
    ContextMenuState, DashboardState, MouseAction, TuiPanel,
};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::style::Color;
use ratatui::Terminal;

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
        vec!["example/alpha:latest".into(), "example/mingli:latest".into()],
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

    assert!(rendered.contains("DOCKERCTL COMMAND CENTER"));
    assert!(rendered.contains("Projects"));
    assert!(rendered.contains("Risk"));
    assert!(rendered.contains("mingli"));
    assert!(rendered.contains("Ops Deck"));
    assert!(rendered.contains("unhealthy"));
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
    assert!(rendered.contains("dockerctl rescue mingli --dry-run"));
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
    assert_eq!(mouse_action_for_event(header_click, (110, 32), 1, None), None);

    let footer_click = mouse(MouseEventKind::Down(MouseButton::Left), 20, 31);
    assert_eq!(mouse_action_for_event(footer_click, (110, 32), 1, None), None);
}
