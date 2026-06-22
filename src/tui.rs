use std::collections::BTreeSet;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size as terminal_size, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, ListState, Paragraph, Row, Table, TableState, Wrap,
};
use ratatui::Terminal;

use crate::docker::DockerClient;
use crate::domain::{DockerSnapshot, OperationAction, Project, SortMode};
use crate::health::{analyze_snapshot, global_findings};
use crate::ops::{OperationPlan, OperationPlanner};
use crate::{msg, AppResult};

const HEADER_ROWS: u16 = 3;
const METRIC_ROWS: u16 = 5;
const FOOTER_ROWS: u16 = 3;
const PROJECT_HEADER_ROWS: u16 = 2;
const CONTEXT_MENU_WIDTH: u16 = 36;
const CONTEXT_MENU_ITEMS: [ContextMenuItem; 8] = [
    ContextMenuItem::Inspect,
    ContextMenuItem::Doctor,
    ContextMenuItem::Start,
    ContextMenuItem::Stop,
    ContextMenuItem::Restart,
    ContextMenuItem::Rescue,
    ContextMenuItem::Remove,
    ContextMenuItem::Purge,
];

pub async fn run(client: DockerClient) -> AppResult<()> {
    let mut app = TuiApp::new(client).await?;
    let terminal = TerminalSession::enter()?;
    app.run(terminal).await
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalSession {
    fn enter() -> AppResult<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiPanel {
    Detail,
    Doctor,
    Plan(OperationAction),
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMenuState {
    pub project: String,
    pub row: usize,
    pub x: u16,
    pub y: u16,
    pub selected_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPrompt {
    pub action: OperationAction,
    pub token_input: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    Inspect,
    Doctor,
    Start,
    Stop,
    Restart,
    Rescue,
    Remove,
    Purge,
}

impl ContextMenuItem {
    fn label(self) -> &'static str {
        match self {
            Self::Inspect => "Inspect",
            Self::Doctor => "Doctor",
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Restart => "Restart",
            Self::Rescue => "Rescue",
            Self::Remove => "Remove",
            Self::Purge => "Purge",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Inspect => "details",
            Self::Doctor => "diagnose",
            Self::Start => "plan start",
            Self::Stop => "plan stop",
            Self::Restart => "plan restart",
            Self::Rescue => "restart risky",
            Self::Remove => "confirm remove",
            Self::Purge => "confirm purge",
        }
    }

    fn panel(self) -> TuiPanel {
        match self {
            Self::Inspect => TuiPanel::Detail,
            Self::Doctor => TuiPanel::Doctor,
            Self::Start => TuiPanel::Plan(OperationAction::Start),
            Self::Stop => TuiPanel::Plan(OperationAction::Stop),
            Self::Restart => TuiPanel::Plan(OperationAction::Restart),
            Self::Rescue => TuiPanel::Plan(OperationAction::Rescue),
            Self::Remove => TuiPanel::Plan(OperationAction::Remove),
            Self::Purge => TuiPanel::Plan(OperationAction::Purge),
        }
    }
}

pub struct DashboardState {
    pub snapshot: DockerSnapshot,
    pub filtered: Vec<Project>,
    pub selected: BTreeSet<String>,
    pub table_state: TableState,
    pub filter: String,
    pub running_only: bool,
    pub sort_mode: SortMode,
    pub panel: TuiPanel,
    pub status: String,
    pub context_menu: Option<ContextMenuState>,
    pub execution_prompt: Option<ExecutionPrompt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    ProjectRowClick { row: usize },
    PanelClick { slot: usize },
    OpenContextMenu { row: usize, x: u16, y: u16 },
    ContextMenuClick { item: ContextMenuItem },
    ContextMenuHover { item: ContextMenuItem },
    CloseContextMenu,
    ScrollUp,
    ScrollDown,
}

impl DashboardState {
    pub fn from_snapshot(snapshot: DockerSnapshot, sort_mode: SortMode) -> Self {
        let mut state = Self {
            snapshot,
            filtered: Vec::new(),
            selected: BTreeSet::new(),
            table_state: TableState::default(),
            filter: String::new(),
            running_only: false,
            sort_mode,
            panel: TuiPanel::Detail,
            status: String::new(),
            context_menu: None,
            execution_prompt: None,
        };
        state.rebuild_filtered();
        state
    }

    pub fn rebuild_filtered(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .snapshot
            .projects_sorted(self.sort_mode)
            .into_iter()
            .filter(|project| !self.running_only || project.active() > 0)
            .filter(|project| needle.is_empty() || project.name.to_lowercase().contains(&needle))
            .collect();
        self.selected
            .retain(|name| self.filtered.iter().any(|project| &project.name == name));
        if self.filtered.is_empty() {
            self.table_state.select(None);
        } else if self.table_state.selected().is_none() {
            self.table_state.select(Some(0));
        } else if let Some(index) = self.table_state.selected() {
            self.table_state.select(Some(index.min(self.filtered.len() - 1)));
        }
    }

    pub fn current_project(&self) -> Option<&Project> {
        self.table_state
            .selected()
            .and_then(|index| self.filtered.get(index))
    }

    pub fn next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let next = self
            .table_state
            .selected()
            .map(|index| (index + 1).min(self.filtered.len() - 1))
            .unwrap_or(0);
        self.table_state.select(Some(next));
    }

    pub fn previous(&mut self) {
        let previous = self
            .table_state
            .selected()
            .map(|index| index.saturating_sub(1))
            .unwrap_or(0);
        self.table_state.select(Some(previous));
    }

    fn action_targets(&self) -> Vec<String> {
        if self.selected.is_empty() {
            return self
                .current_project()
                .map(|project| vec![project.name.clone()])
                .unwrap_or_default();
        }
        self.filtered
            .iter()
            .filter(|project| self.selected.contains(&project.name))
            .map(|project| project.name.clone())
            .collect()
    }

    fn plan_for(&self, action: OperationAction) -> AppResult<OperationPlan> {
        OperationPlanner::new(&self.snapshot).plan(action, &self.action_targets())
    }
}

pub fn begin_execution_prompt(state: &mut DashboardState) {
    let TuiPanel::Plan(action) = state.panel else {
        return;
    };
    state.context_menu = None;
    state.execution_prompt = Some(ExecutionPrompt {
        action,
        token_input: String::new(),
    });
    state.status = match action {
        OperationAction::Remove | OperationAction::Purge | OperationAction::Prune => {
            "输入确认令牌后按 Enter 执行，Esc 取消。".to_string()
        }
        _ => format!("再次按 Enter 执行 {}，Esc 取消。", operation_label(action)),
    };
}

pub fn cancel_execution_prompt(state: &mut DashboardState) {
    state.execution_prompt = None;
    state.status = "已取消 TUI 执行。".to_string();
}

pub fn push_execution_token(state: &mut DashboardState, ch: char) {
    if let Some(prompt) = state.execution_prompt.as_mut() {
        prompt.token_input.push(ch);
    }
}

pub fn pop_execution_token(state: &mut DashboardState) {
    if let Some(prompt) = state.execution_prompt.as_mut() {
        prompt.token_input.pop();
    }
}

pub fn execution_plan_if_confirmed(state: &DashboardState) -> AppResult<Option<OperationPlan>> {
    let Some(prompt) = state.execution_prompt.as_ref() else {
        return Ok(None);
    };
    let plan = state.plan_for(prompt.action)?;
    if let Some(token) = plan.confirmation_token.as_deref() {
        if prompt.token_input == token {
            return Ok(Some(plan));
        }
        return Ok(None);
    }
    Ok(Some(plan))
}

pub fn apply_mouse_action(state: &mut DashboardState, action: MouseAction) {
    match action {
        MouseAction::ProjectRowClick { row } => {
            state.context_menu = None;
            state.execution_prompt = None;
            if row < state.filtered.len() {
                state.table_state.select(Some(row));
                let name = state.filtered[row].name.clone();
                if !state.selected.insert(name.clone()) {
                    state.selected.remove(&name);
                }
            }
        }
        MouseAction::PanelClick { slot } => {
            state.context_menu = None;
            state.execution_prompt = None;
            state.panel = match slot {
                0 => TuiPanel::Detail,
                1 => TuiPanel::Doctor,
                _ => TuiPanel::Help,
            };
        }
        MouseAction::OpenContextMenu { row, x, y } => {
            if row < state.filtered.len() {
                state.execution_prompt = None;
                state.table_state.select(Some(row));
                let name = state.filtered[row].name.clone();
                if state.selected.is_empty() || !state.selected.contains(&name) {
                    state.selected.clear();
                    state.selected.insert(name.clone());
                }
                state.context_menu = Some(ContextMenuState {
                    project: name.clone(),
                    row,
                    x,
                    y,
                    selected_index: 0,
                });
                state.status = format!("右键管理菜单已打开: {name}");
            }
        }
        MouseAction::ContextMenuHover { item } => {
            if let Some(menu) = state.context_menu.as_mut() {
                menu.selected_index = context_menu_item_index(item);
            }
        }
        MouseAction::ContextMenuClick { item } => {
            state.panel = item.panel();
            state.context_menu = None;
            state.execution_prompt = None;
            state.status = format!("已选择右键菜单动作: {}", item.label());
        }
        MouseAction::CloseContextMenu => {
            state.context_menu = None;
        }
        MouseAction::ScrollUp => {
            state.context_menu = None;
            state.execution_prompt = None;
            state.previous();
        }
        MouseAction::ScrollDown => {
            state.context_menu = None;
            state.execution_prompt = None;
            state.next();
        }
    }
}

struct TuiApp {
    client: DockerClient,
    snapshot: DockerSnapshot,
    filtered: Vec<Project>,
    selected: BTreeSet<String>,
    list_state: ListState,
    filter: String,
    running_only: bool,
    sort_mode: SortMode,
    panel: TuiPanel,
    status: String,
    context_menu: Option<ContextMenuState>,
    execution_prompt: Option<ExecutionPrompt>,
    last_refresh: Instant,
}

impl TuiApp {
    async fn new(client: DockerClient) -> AppResult<Self> {
        let snapshot = client.snapshot().await?;
        let mut app = Self {
            client,
            snapshot,
            filtered: Vec::new(),
            selected: BTreeSet::new(),
            list_state: ListState::default(),
            filter: String::new(),
            running_only: false,
            sort_mode: SortMode::Severity,
            panel: TuiPanel::Detail,
            status: String::new(),
            context_menu: None,
            execution_prompt: None,
            last_refresh: Instant::now(),
        };
        app.rebuild_filtered();
        Ok(app)
    }

    async fn run(&mut self, mut session: TerminalSession) -> AppResult<()> {
        loop {
            session.terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(80))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press && self.handle_key(key.code).await? {
                            return Ok(());
                        }
                    }
                    Event::Mouse(mouse) => self.handle_mouse(mouse)?,
                    _ => {}
                }
            }
            if self.last_refresh.elapsed() >= Duration::from_secs(2) {
                self.refresh().await?;
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> AppResult<()> {
        let Some(action) = mouse_action_for_event(
            mouse,
            terminal_size()?,
            self.filtered.len(),
            self.context_menu.as_ref(),
        ) else {
            return Ok(());
        };
        let mut state = DashboardState {
            snapshot: self.snapshot.clone(),
            filtered: self.filtered.clone(),
            selected: self.selected.clone(),
            table_state: TableState::default(),
            filter: self.filter.clone(),
            running_only: self.running_only,
            sort_mode: self.sort_mode,
            panel: self.panel,
            status: self.status.clone(),
            context_menu: self.context_menu.clone(),
            execution_prompt: self.execution_prompt.clone(),
        };
        state.table_state.select(self.list_state.selected());
        apply_mouse_action(&mut state, action);
        self.selected = state.selected;
        self.panel = state.panel;
        self.status = state.status;
        self.context_menu = state.context_menu;
        self.execution_prompt = state.execution_prompt;
        self.list_state.select(state.table_state.selected());
        Ok(())
    }

    async fn handle_key(&mut self, code: KeyCode) -> AppResult<bool> {
        if self.execution_prompt.is_some() {
            return self.handle_execution_key(code).await;
        }
        if self.context_menu.is_some() {
            return self.handle_context_menu_key(code);
        }
        match code {
            KeyCode::Esc if self.context_menu.is_some() => {
                self.context_menu = None;
                return Ok(false);
            }
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => {
                self.context_menu = None;
                self.next();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.context_menu = None;
                self.previous();
            }
            KeyCode::Char(' ') => {
                self.context_menu = None;
                self.toggle_selected();
            }
            KeyCode::Char('a') => {
                self.context_menu = None;
                self.toggle_all();
            }
            KeyCode::Char('c') => {
                self.context_menu = None;
                self.selected.clear();
            }
            KeyCode::Char('r') => self.refresh().await?,
            KeyCode::Char('x') => {
                self.context_menu = None;
                self.running_only = !self.running_only;
                self.rebuild_filtered();
            }
            KeyCode::Char('o') => {
                self.context_menu = None;
                self.sort_mode = match self.sort_mode {
                    SortMode::Severity => SortMode::NameAsc,
                    SortMode::NameAsc => SortMode::ActiveDesc,
                    SortMode::ActiveDesc => SortMode::Severity,
                };
                self.rebuild_filtered();
            }
            KeyCode::Char('/') => {
                self.context_menu = None;
                self.status = "输入过滤字符；退格删除，Enter 确认，Esc 清空。".to_string();
            }
            KeyCode::Backspace => {
                self.context_menu = None;
                self.filter.pop();
                self.rebuild_filtered();
            }
            KeyCode::Enter => {
                self.context_menu = None;
                if matches!(self.panel, TuiPanel::Plan(_)) {
                    let mut state = self.dashboard_state();
                    begin_execution_prompt(&mut state);
                    self.status = state.status;
                    self.execution_prompt = state.execution_prompt;
                } else {
                    self.panel = TuiPanel::Plan(OperationAction::Stop);
                }
            }
            KeyCode::Char('1') => {
                self.context_menu = None;
                self.panel = TuiPanel::Plan(OperationAction::Start);
            }
            KeyCode::Char('2') => {
                self.context_menu = None;
                self.panel = TuiPanel::Plan(OperationAction::Stop);
            }
            KeyCode::Char('3') => {
                self.context_menu = None;
                self.panel = TuiPanel::Plan(OperationAction::Restart);
            }
            KeyCode::Char('4') => {
                self.context_menu = None;
                self.panel = TuiPanel::Plan(OperationAction::Remove);
            }
            KeyCode::Char('5') => {
                self.context_menu = None;
                self.panel = TuiPanel::Plan(OperationAction::Purge);
            }
            KeyCode::Char('d') => {
                self.context_menu = None;
                self.panel = TuiPanel::Doctor;
            }
            KeyCode::Char('i') => {
                self.context_menu = None;
                self.panel = TuiPanel::Detail;
            }
            KeyCode::Char('h') | KeyCode::Char('?') => {
                self.context_menu = None;
                self.panel = TuiPanel::Help;
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.context_menu = None;
                self.filter.push(ch);
                self.rebuild_filtered();
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_context_menu_key(&mut self, code: KeyCode) -> AppResult<bool> {
        match code {
            KeyCode::Esc => self.context_menu = None,
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(menu) = self.context_menu.as_mut() {
                    menu.selected_index = menu.selected_index.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(menu) = self.context_menu.as_mut() {
                    menu.selected_index =
                        (menu.selected_index + 1).min(CONTEXT_MENU_ITEMS.len() - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(item) = self.context_menu.as_ref().map(context_menu_selected_item) {
                    let mut state = self.dashboard_state();
                    apply_mouse_action(&mut state, MouseAction::ContextMenuClick { item });
                    self.panel = state.panel;
                    self.status = state.status;
                    self.context_menu = state.context_menu;
                    self.execution_prompt = state.execution_prompt;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    async fn handle_execution_key(&mut self, code: KeyCode) -> AppResult<bool> {
        let mut state = self.dashboard_state();
        match code {
            KeyCode::Esc => {
                cancel_execution_prompt(&mut state);
                self.status = state.status;
                self.execution_prompt = state.execution_prompt;
            }
            KeyCode::Backspace => {
                pop_execution_token(&mut state);
                self.execution_prompt = state.execution_prompt;
            }
            KeyCode::Enter => {
                self.execute_confirmed_plan(state).await?;
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                push_execution_token(&mut state, ch);
                self.execution_prompt = state.execution_prompt;
            }
            _ => {}
        }
        Ok(false)
    }

    async fn execute_confirmed_plan(&mut self, state: DashboardState) -> AppResult<()> {
        let Some(plan) = execution_plan_if_confirmed(&state)? else {
            self.status = "确认令牌未匹配，继续输入或按 Esc 取消。".to_string();
            self.execution_prompt = state.execution_prompt;
            return Ok(());
        };
        let action = plan.action;
        self.status = format!("正在执行 {} ...", operation_label(action));
        let result = self.client.execute_plan(&plan, false).await?;
        self.execution_prompt = None;
        self.status = format!(
            "{} 执行完成: 成功 {} 个，失败 {} 个。",
            operation_label(action),
            result.success.len(),
            result.failed.len()
        );
        self.refresh().await?;
        Ok(())
    }

    async fn refresh(&mut self) -> AppResult<()> {
        self.snapshot = self.client.snapshot().await?;
        self.last_refresh = Instant::now();
        self.rebuild_filtered();
        Ok(())
    }
    fn rebuild_filtered(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .snapshot
            .projects_sorted(self.sort_mode)
            .into_iter()
            .filter(|project| !self.running_only || project.active() > 0)
            .filter(|project| needle.is_empty() || project.name.to_lowercase().contains(&needle))
            .collect();
        self.selected
            .retain(|name| self.filtered.iter().any(|project| &project.name == name));
        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else if self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        } else if let Some(index) = self.list_state.selected() {
            self.list_state.select(Some(index.min(self.filtered.len() - 1)));
        }
    }

    fn next(&mut self) {
        if self.filtered.is_empty() {
            return;
        }
        let next = self
            .list_state
            .selected()
            .map(|index| (index + 1).min(self.filtered.len() - 1))
            .unwrap_or(0);
        self.list_state.select(Some(next));
    }

    fn previous(&mut self) {
        let previous = self
            .list_state
            .selected()
            .map(|index| index.saturating_sub(1))
            .unwrap_or(0);
        self.list_state.select(Some(previous));
    }

    fn toggle_selected(&mut self) {
        let Some(project) = self.current_project() else {
            return;
        };
        let name = project.name.clone();
        if !self.selected.insert(name.clone()) {
            self.selected.remove(&name);
        }
    }

    fn toggle_all(&mut self) {
        if !self.filtered.is_empty()
            && self
                .filtered
                .iter()
                .all(|project| self.selected.contains(&project.name))
        {
            self.selected.clear();
        } else {
            self.selected = self.filtered.iter().map(|project| project.name.clone()).collect();
        }
    }

    fn current_project(&self) -> Option<&Project> {
        self.list_state
            .selected()
            .and_then(|index| self.filtered.get(index))
    }

    fn dashboard_state(&self) -> DashboardState {
        let mut state = DashboardState {
            snapshot: self.snapshot.clone(),
            filtered: self.filtered.clone(),
            selected: self.selected.clone(),
            table_state: TableState::default(),
            filter: self.filter.clone(),
            running_only: self.running_only,
            sort_mode: self.sort_mode,
            panel: self.panel,
            status: self.status.clone(),
            context_menu: self.context_menu.clone(),
            execution_prompt: self.execution_prompt.clone(),
        };
        state.table_state.select(self.list_state.selected());
        state
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let mut state = self.dashboard_state();
        render_dashboard(frame, &mut state);
    }
}

pub fn render_dashboard(frame: &mut ratatui::Frame, state: &mut DashboardState) {
    let area = frame.area();
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(area);

    render_header(frame, outer[0], state);
    render_metric_bar(frame, outer[1], state);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(outer[2]);
    render_projects_table(frame, main[0], state);
    render_ops_deck(frame, main[1], state);
    render_command_bar(frame, outer[3], state);
    render_context_menu(frame, area, state);
}

fn render_header(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let filter = if state.filter.is_empty() {
        "none"
    } else {
        &state.filter
    };
    let title = Line::from(vec![
        Span::styled(
            " DOCKERCTL COMMAND CENTER ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" project-first docker ops ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(
                " mode:{} sort:{:?} filter:{} ",
                if state.running_only { "active" } else { "all" },
                state.sort_mode,
                filter
            ),
            Style::default().fg(Color::Yellow),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan)),
            ),
        area,
    );
}

fn render_metric_bar(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let metrics = dashboard_metrics(state);
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(area);

    for (index, (label, value, color)) in metrics.into_iter().enumerate() {
        let text = Line::from(vec![
            Span::styled(value, Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(label, Style::default().fg(Color::DarkGray)),
        ]);
        frame.render_widget(
            Paragraph::new(text).alignment(Alignment::Center).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(color)),
            ),
            chunks[index],
        );
    }
}

fn render_projects_table(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &mut DashboardState,
) {
    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("State"),
        Cell::from("Project"),
        Cell::from("Kind"),
        Cell::from("Run"),
        Cell::from("Risk"),
    ])
    .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

    let rows = state.filtered.iter().map(|project| {
        let is_selected = state.selected.contains(&project.name);
        let selected = if is_selected { "[x]" } else { "[ ]" };
        let risk = project_risk(project);
        let marker_style = if is_selected {
            selected_project_style()
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let project_name_style = if is_selected {
            selected_project_style()
        } else {
            Style::default().fg(Color::White)
        };
        let row = Row::new(vec![
            Cell::from(selected).style(marker_style),
            Cell::from(project.state_code()).style(project_style(project)),
            Cell::from(project.name.clone()).style(project_name_style),
            Cell::from(format!("{:?}", project.kind)).style(Style::default().fg(Color::DarkGray)),
            Cell::from(format!("{}/{}", project.active(), project.containers.len())),
            Cell::from(risk.0).style(Style::default().fg(risk.1).add_modifier(Modifier::BOLD)),
        ]);
        if is_selected {
            row.style(Style::default().bg(Color::Rgb(43, 36, 11)))
        } else {
            row
        }
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(6),
            Constraint::Min(14),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Projects")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Blue)),
    )
    .row_highlight_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut state.table_state);
}

fn render_ops_deck(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let title = match state.panel {
        TuiPanel::Detail => "Ops Deck / Detail",
        TuiPanel::Doctor => "Ops Deck / Doctor",
        TuiPanel::Plan(OperationAction::Start) => "Ops Deck / Plan Start",
        TuiPanel::Plan(OperationAction::Stop) => "Ops Deck / Plan Stop",
        TuiPanel::Plan(OperationAction::Restart) => "Ops Deck / Plan Restart",
        TuiPanel::Plan(OperationAction::Remove) => "Ops Deck / Plan Remove",
        TuiPanel::Plan(OperationAction::Purge) => "Ops Deck / Plan Purge",
        TuiPanel::Plan(OperationAction::Prune) => "Ops Deck / Plan Prune",
        TuiPanel::Plan(OperationAction::Rescue) => "Ops Deck / Plan Rescue",
        TuiPanel::Help => "Ops Deck / Help",
    };
    frame.render_widget(
        Paragraph::new(panel_text(state))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Magenta)),
            ),
        area,
    );
}

fn render_command_bar(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let text = if state.status.is_empty() {
        " mouse: click row select, right-click manage, wheel move | keys: j/k move | space select | / filter | i detail | d doctor | 1-5 plan | Enter execute | q quit "
            .to_string()
    } else {
        state.status.clone()
    };
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title("Command Bar")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
        area,
    );
}

fn render_context_menu(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let Some(menu) = state.context_menu.as_ref() else {
        return;
    };
    let rect = context_menu_rect(area, menu);
    let title = if state.selected.len() > 1 && state.selected.contains(&menu.project) {
        format!("Manage {} selected", state.selected.len())
    } else {
        format!("Manage {}", menu.project)
    };
    let lines = CONTEXT_MENU_ITEMS
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let selected = index == menu.selected_index;
            Line::from(vec![
                Span::styled(
                    format!(" {:<8}", item.label()),
                    context_menu_item_style(*item, selected),
                ),
                Span::styled(
                    item.description(),
                    context_menu_description_style(selected),
                ),
            ])
        })
        .collect::<Vec<_>>();

    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        rect,
    );
}

pub fn mouse_action_for_event(
    mouse: MouseEvent,
    terminal_size: (u16, u16),
    visible_projects: usize,
    context_menu: Option<&ContextMenuState>,
) -> Option<MouseAction> {
    let (cols, rows) = terminal_size;
    let screen = Rect::new(0, 0, cols, rows);

    if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
        if let Some(menu) = context_menu {
            if let Some(item) = context_menu_item_at(screen, menu, mouse.column, mouse.row) {
                return Some(MouseAction::ContextMenuClick { item });
            }
            return Some(MouseAction::CloseContextMenu);
        }
    }
    if matches!(mouse.kind, MouseEventKind::Moved | MouseEventKind::Drag(_)) {
        if let Some(menu) = context_menu {
            return context_menu_item_at(screen, menu, mouse.column, mouse.row)
                .map(|item| MouseAction::ContextMenuHover { item });
        }
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => return Some(MouseAction::ScrollUp),
        MouseEventKind::ScrollDown => return Some(MouseAction::ScrollDown),
        MouseEventKind::Down(MouseButton::Left) => {}
        MouseEventKind::Down(MouseButton::Right) => {
            return project_row_for_mouse(mouse, terminal_size, visible_projects)
                .map(|row| MouseAction::OpenContextMenu {
                    row,
                    x: mouse.column,
                    y: mouse.row,
                })
                .or_else(|| context_menu.map(|_| MouseAction::CloseContextMenu));
        }
        _ => return None,
    }

    if let Some(row) = project_row_for_mouse(mouse, terminal_size, visible_projects) {
        return Some(MouseAction::ProjectRowClick { row });
    }

    if !is_in_main_area(mouse, terminal_size) {
        return None;
    }

    None
}

fn is_in_main_area(mouse: MouseEvent, terminal_size: (u16, u16)) -> bool {
    let (_, rows) = terminal_size;
    if rows <= HEADER_ROWS + METRIC_ROWS + FOOTER_ROWS {
        return false;
    }
    let main_y = HEADER_ROWS + METRIC_ROWS;
    let main_height = rows.saturating_sub(HEADER_ROWS + METRIC_ROWS + FOOTER_ROWS);
    let main_bottom = main_y + main_height;
    mouse.row >= main_y && mouse.row < main_bottom
}

fn project_row_for_mouse(
    mouse: MouseEvent,
    terminal_size: (u16, u16),
    visible_projects: usize,
) -> Option<usize> {
    let (cols, rows) = terminal_size;
    if rows <= HEADER_ROWS + METRIC_ROWS + FOOTER_ROWS {
        return None;
    }
    let main_y = HEADER_ROWS + METRIC_ROWS;
    let main_height = rows.saturating_sub(HEADER_ROWS + METRIC_ROWS + FOOTER_ROWS);
    let main_bottom = main_y + main_height;
    if mouse.row < main_y || mouse.row >= main_bottom {
        return None;
    }

    let left_width = ((cols as u32 * 48) / 100).max(1) as u16;
    if mouse.column >= left_width {
        return None;
    }

    let row = mouse.row.saturating_sub(main_y + PROJECT_HEADER_ROWS) as usize;
    (row < visible_projects).then_some(row)
}

fn context_menu_rect(area: Rect, menu: &ContextMenuState) -> Rect {
    let width = CONTEXT_MENU_WIDTH.min(area.width.max(1));
    let height = ((CONTEXT_MENU_ITEMS.len() + 2) as u16).min(area.height.max(1));
    let max_x = area.x + area.width.saturating_sub(width);
    let max_y = area.y + area.height.saturating_sub(height);
    Rect::new(menu.x.min(max_x), menu.y.min(max_y), width, height)
}

fn context_menu_item_at(
    area: Rect,
    menu: &ContextMenuState,
    column: u16,
    row: u16,
) -> Option<ContextMenuItem> {
    let rect = context_menu_rect(area, menu);
    if column <= rect.x
        || column >= rect.x + rect.width.saturating_sub(1)
        || row <= rect.y
        || row >= rect.y + rect.height.saturating_sub(1)
    {
        return None;
    }
    let item_index = row.saturating_sub(rect.y + 1) as usize;
    CONTEXT_MENU_ITEMS.get(item_index).copied()
}

fn context_menu_item_style(item: ContextMenuItem, selected: bool) -> Style {
    if selected {
        return Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
    }
    match item {
        ContextMenuItem::Remove => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        ContextMenuItem::Purge => Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::BOLD),
        ContextMenuItem::Rescue => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::White),
    }
}

fn context_menu_description_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn context_menu_item_index(item: ContextMenuItem) -> usize {
    CONTEXT_MENU_ITEMS
        .iter()
        .position(|candidate| *candidate == item)
        .unwrap_or(0)
}

fn context_menu_selected_item(menu: &ContextMenuState) -> ContextMenuItem {
    CONTEXT_MENU_ITEMS
        .get(menu.selected_index)
        .copied()
        .unwrap_or(ContextMenuItem::Inspect)
}

fn dashboard_metrics(state: &DashboardState) -> Vec<(&'static str, String, Color)> {
    let total = state.snapshot.projects.len();
    let active = state
        .snapshot
        .projects
        .iter()
        .filter(|project| project.active() > 0)
        .count();
    let unhealthy = state
        .snapshot
        .projects
        .iter()
        .filter(|project| project.unhealthy > 0 || project.restarting > 0)
        .count();
    let selected = state.selected.len();
    let risk_color = if unhealthy > 0 { Color::Red } else { Color::Green };
    vec![
        ("Projects", total.to_string(), Color::Cyan),
        ("Active", active.to_string(), Color::Green),
        ("Risk", unhealthy.to_string(), risk_color),
        ("Selected", selected.to_string(), Color::Yellow),
        (
            "Visible",
            state.filtered.len().to_string(),
            if state.running_only {
                Color::Blue
            } else {
                Color::DarkGray
            },
        ),
    ]
}

fn panel_text(state: &DashboardState) -> String {
    match state.panel {
        TuiPanel::Detail => detail_text(state),
        TuiPanel::Doctor => doctor_text(&state.snapshot),
        TuiPanel::Plan(action) => state
            .plan_for(action)
            .map(|plan| format_plan(plan, state.execution_prompt.as_ref()))
            .unwrap_or_else(|err| err.to_string()),
        TuiPanel::Help => help_text(),
    }
}

fn detail_text(state: &DashboardState) -> String {
    let Some(project) = state.current_project() else {
        return "No project matches current filter.".to_string();
    };
    let mut text = format!(
        "{}\nkind: {:?}\nstate: {}\ncontainers: {} active:{} stopped:{}\nnetworks: {}\nvolumes: {}\nimages: {}\nports: {}\n\n",
        project.name,
        project.kind,
        project.state_code(),
        project.containers.len(),
        project.active(),
        project.stopped,
        project.networks.join(", "),
        project.volumes.join(", "),
        project.images.join(", "),
        project.ports.join(", ")
    );
    for container in &project.containers {
        text.push_str(&format!(
            "- {} [{}] {}\n  {}\n",
            container.name,
            container.state.state_code(),
            container.image,
            container.status
        ));
    }
    text
}

fn doctor_text(snapshot: &DockerSnapshot) -> String {
    let mut text = String::new();
    for health in analyze_snapshot(snapshot) {
        text.push_str(&format!("{:?} {}\n", health.status, health.project));
        for finding in health.findings {
            text.push_str(&format!("  - {finding}\n"));
        }
    }
    for finding in global_findings(snapshot) {
        text.push_str(&format!("global: {finding}\n"));
    }
    if text.is_empty() {
        "No obvious risk found.".to_string()
    } else {
        text
    }
}

fn project_style(project: &Project) -> Style {
    if project.unhealthy > 0 {
        Style::default().fg(Color::Red)
    } else if project.restarting > 0 {
        Style::default().fg(Color::Yellow)
    } else if project.paused > 0 {
        Style::default().fg(Color::Blue)
    } else if project.active() > 0 {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn selected_project_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn project_risk(project: &Project) -> (&'static str, Color) {
    if project.unhealthy > 0 {
        ("HIGH", Color::Red)
    } else if project.restarting > 0 {
        ("LOOP", Color::Yellow)
    } else if project.paused > 0 {
        ("PAUSE", Color::Blue)
    } else if project.active() > 0 {
        ("LOW", Color::Green)
    } else {
        ("IDLE", Color::DarkGray)
    }
}

fn format_plan(plan: OperationPlan, prompt: Option<&ExecutionPrompt>) -> String {
    let mut text = format!("{}\n\n项目: {}\n", plan.summary, plan.projects.join(", "));
    if !plan.containers.is_empty() {
        text.push_str(&format!("容器: {}\n", plan.containers.join(", ")));
    }
    if !plan.networks.is_empty() {
        text.push_str(&format!("网络: {}\n", plan.networks.join(", ")));
    }
    if !plan.volumes.is_empty() {
        text.push_str(&format!("卷: {}\n", plan.volumes.join(", ")));
    }
    if !plan.images.is_empty() {
        text.push_str(&format!("镜像: {}\n", plan.images.join(", ")));
    }
    for warning in &plan.warnings {
        text.push_str(&format!("警告: {warning}\n"));
    }
    if plan.action == OperationAction::Rescue {
        text.push_str(&format_rescue_playbook(&plan));
    }
    if let Some(token) = &plan.confirmation_token {
        text.push_str(&format!("\nCLI 执行需确认令牌: {token}\n"));
    }
    if is_destructive_action(plan.action) {
        text.push_str(&format_safety_rail(&plan));
    }
    text.push_str(&format_execution_prompt(&plan, prompt));
    text
}

fn format_rescue_playbook(plan: &OperationPlan) -> String {
    let target = plan.projects.join(" ");
    format!(
         "\nRecovery Playbook\n\
         异常信号: 优先处理 unhealthy / restarting / active 容器。\n\
         执行策略: 先生成恢复重启预案，TUI 中需二次确认后才执行。\n\
         验证命令: dockerctl rescue {target} --dry-run\n\
         执行命令: dockerctl rescue {target}\n\
         回滚提示: 若恢复后仍异常，先查看 dockerctl logs 和 doctor 输出，再考虑 remove/purge。\n"
    )
}

fn format_safety_rail(plan: &OperationPlan) -> String {
    let token = plan
        .confirmation_token
        .as_deref()
        .unwrap_or("required-token");
    format!(
        "\nSafety Rail\n\
         destructive action: {}\n\
         required token: {token}\n\
         mouse cannot execute destructive actions\n\
         use Enter confirmation only after reviewing containers/networks/volumes/images above\n",
        operation_label(plan.action)
    )
}

fn format_execution_prompt(plan: &OperationPlan, prompt: Option<&ExecutionPrompt>) -> String {
    let Some(prompt) = prompt.filter(|prompt| prompt.action == plan.action) else {
        return "\nTUI 执行: 按 Enter 打开执行确认。\n".to_string();
    };
    let title = format!("\nExecute {}\n", operation_label(plan.action));
    if let Some(token) = plan.confirmation_token.as_deref() {
        return format!(
            "{title}确认令牌: {token}\n已输入: {}\n输入完整令牌后按 Enter 执行；Esc to cancel。\n",
            prompt.token_input
        );
    }
    format!("{title}Enter again to execute; Esc to cancel.\n")
}

fn operation_label(action: OperationAction) -> &'static str {
    match action {
        OperationAction::Start => "Start",
        OperationAction::Stop => "Stop",
        OperationAction::Restart => "Restart",
        OperationAction::Remove => "Remove",
        OperationAction::Purge => "Purge",
        OperationAction::Prune => "Prune",
        OperationAction::Rescue => "Rescue",
    }
}

fn is_destructive_action(action: OperationAction) -> bool {
    matches!(
        action,
        OperationAction::Remove | OperationAction::Purge | OperationAction::Prune
    )
}

fn help_text() -> String {
    [
        "dockerctl TUI",
        "",
        "鼠标左键项目行: 选择/反选",
        "鼠标右键项目行: 打开管理菜单",
        "鼠标滚轮: 移动项目",
        "j/k 或 ↑/↓: 移动",
        "space: 多选；a: 全选/反选；c: 清空选择",
        "/: 输入过滤；Backspace 删除过滤字符",
        "x: 仅活动项目；o: 切换排序；r: 刷新",
        "i: 详情；d: doctor；1/2/3/4/5: start/stop/restart/remove/purge 预演",
        "Enter: 在计划面板打开执行确认；确认中再次 Enter 执行",
        "q/Esc: 退出；确认中 Esc 取消执行",
        "",
        "Remove/Purge 必须输入确认令牌；普通动作需要二次 Enter。",
    ]
    .join("\n")
}

#[allow(dead_code)]
fn ensure_non_empty_projects(snapshot: &DockerSnapshot) -> AppResult<()> {
    if snapshot.projects.is_empty() {
        msg("未发现 Docker 项目。")
    } else {
        Ok(())
    }
}
