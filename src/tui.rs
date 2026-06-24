use std::collections::BTreeSet;
use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    size as terminal_size,
};
use futures_util::FutureExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, ListState, Paragraph, Row, Table, TableState, Wrap,
};
use tokio::task::JoinHandle;

use crate::config::{ThemeName, parse_theme};
use crate::docker::DockerClient;
use crate::domain::{DockerSnapshot, OperationAction, Project, SortMode};
use crate::health::{analyze_snapshot, global_findings};
use crate::inbox::{InboxItem, InboxSeverity, build_ops_inbox};
use crate::ops::{OperationPlan, OperationPlanner};
use crate::resources::{
    ResourcePanelData, ResourceRow, ResourceTrend, format_bytes, format_signed_bytes,
    resource_pressure_hint, resource_trend, sorted_resource_rows,
};
use crate::{AppResult, msg};

const HEADER_ROWS: u16 = 3;
const METRIC_ROWS: u16 = 5;
const FOOTER_ROWS: u16 = 3;
const PROJECT_HEADER_ROWS: u16 = 2;
const CONTEXT_MENU_WIDTH: u16 = 36;
const MIN_TUI_WIDTH: u16 = 90;
const MIN_TUI_HEIGHT: u16 = 22;
const CONTEXT_MENU_ITEMS: [ContextMenuItem; 10] = [
    ContextMenuItem::Inspect,
    ContextMenuItem::Doctor,
    ContextMenuItem::Start,
    ContextMenuItem::Stop,
    ContextMenuItem::Restart,
    ContextMenuItem::Rescue,
    ContextMenuItem::Logs,
    ContextMenuItem::Resources,
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
    Inbox,
    Detail,
    Doctor,
    Logs,
    Resources,
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
    Logs,
    Resources,
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
            Self::Logs => "Logs",
            Self::Resources => "Resources",
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
            Self::Logs => "log lens",
            Self::Resources => "resource view",
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
            Self::Logs => TuiPanel::Logs,
            Self::Resources => TuiPanel::Resources,
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
    pub theme: ThemeName,
    pub panel: TuiPanel,
    pub status: String,
    pub context_menu: Option<ContextMenuState>,
    pub execution_prompt: Option<ExecutionPrompt>,
    pub resource_data: Option<ResourcePanelData>,
    pub resource_trend: Option<ResourceTrend>,
    pub log_container_index: usize,
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
            theme: ThemeName::Industrial,
            panel: TuiPanel::Inbox,
            status: String::new(),
            context_menu: None,
            execution_prompt: None,
            resource_data: None,
            resource_trend: None,
            log_container_index: 0,
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
            self.table_state
                .select(Some(index.min(self.filtered.len() - 1)));
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

pub fn mark_resource_refresh_pending(
    current: Option<ResourcePanelData>,
    project: &str,
) -> Option<ResourcePanelData> {
    if project.is_empty() {
        return None;
    }
    match current {
        Some(mut data) if data.project == project && data.sampled_at_ms > 0 => {
            data.stale = true;
            data.loading = false;
            Some(data)
        }
        _ => Some(ResourcePanelData::loading(project.to_string())),
    }
}

fn current_resource_trend(
    previous: Option<&ResourcePanelData>,
    current: Option<&ResourcePanelData>,
) -> Option<ResourceTrend> {
    resource_trend(previous?, current?)
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
                2 => TuiPanel::Logs,
                3 => TuiPanel::Resources,
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
    client: Option<DockerClient>,
    snapshot: DockerSnapshot,
    filtered: Vec<Project>,
    selected: BTreeSet<String>,
    list_state: ListState,
    filter: String,
    running_only: bool,
    sort_mode: SortMode,
    theme: ThemeName,
    panel: TuiPanel,
    status: String,
    context_menu: Option<ContextMenuState>,
    execution_prompt: Option<ExecutionPrompt>,
    resource_data: Option<ResourcePanelData>,
    resource_previous: Option<ResourcePanelData>,
    resource_task: Option<JoinHandle<ResourcePanelData>>,
    resource_refresh_interval: Duration,
    last_refresh: Instant,
    last_resource_refresh: Option<Instant>,
    log_container_index: usize,
}

impl TuiApp {
    async fn new(client: DockerClient) -> AppResult<Self> {
        let theme = parse_theme(&client.config().tui.theme);
        let resource_refresh_interval =
            Duration::from_millis(client.config().tui.refresh_ms.max(250));
        let snapshot = client.snapshot().await?;
        let mut app = Self {
            client: Some(client),
            snapshot,
            filtered: Vec::new(),
            selected: BTreeSet::new(),
            list_state: ListState::default(),
            filter: String::new(),
            running_only: false,
            sort_mode: SortMode::Severity,
            theme,
            panel: TuiPanel::Inbox,
            status: String::new(),
            context_menu: None,
            execution_prompt: None,
            resource_data: None,
            resource_previous: None,
            resource_task: None,
            resource_refresh_interval,
            last_refresh: Instant::now(),
            last_resource_refresh: None,
            log_container_index: 0,
        };
        app.rebuild_filtered();
        Ok(app)
    }

    async fn run(&mut self, mut session: TerminalSession) -> AppResult<()> {
        let mut dirty = true;
        loop {
            dirty |= self.poll_resource_task();
            dirty |= self.ensure_resource_sampling();

            if self.last_refresh.elapsed() >= Duration::from_secs(2) {
                self.refresh().await?;
                dirty = true;
            }

            if dirty {
                session.terminal.draw(|frame| self.draw(frame))?;
                dirty = false;
            }

            if event::poll(Duration::from_millis(80))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press && self.handle_key(key.code).await? {
                            return Ok(());
                        }
                        dirty = true;
                    }
                    Event::Mouse(mouse) => dirty |= self.handle_mouse(mouse)?,
                    _ => {}
                }
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> AppResult<bool> {
        let Some(action) = mouse_action_for_event(
            mouse,
            terminal_size()?,
            self.filtered.len(),
            self.context_menu.as_ref(),
        ) else {
            return Ok(false);
        };
        let mut state = DashboardState {
            snapshot: self.snapshot.clone(),
            filtered: self.filtered.clone(),
            selected: self.selected.clone(),
            table_state: TableState::default(),
            filter: self.filter.clone(),
            running_only: self.running_only,
            sort_mode: self.sort_mode,
            theme: self.theme,
            panel: self.panel,
            status: self.status.clone(),
            context_menu: self.context_menu.clone(),
            execution_prompt: self.execution_prompt.clone(),
            resource_data: self.resource_data.clone(),
            resource_trend: current_resource_trend(
                self.resource_previous.as_ref(),
                self.resource_data.as_ref(),
            ),
            log_container_index: self.log_container_index,
        };
        state.table_state.select(self.list_state.selected());
        apply_mouse_action(&mut state, action);
        self.selected = state.selected;
        self.panel = state.panel;
        self.status = state.status;
        self.context_menu = state.context_menu;
        self.execution_prompt = state.execution_prompt;
        self.resource_data = state.resource_data;
        self.log_container_index = state.log_container_index;
        self.list_state.select(state.table_state.selected());
        Ok(true)
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
            KeyCode::Char('b') => {
                self.context_menu = None;
                self.panel = TuiPanel::Inbox;
            }
            KeyCode::Char('d') => {
                self.context_menu = None;
                self.panel = TuiPanel::Doctor;
            }
            KeyCode::Char('l') => {
                self.context_menu = None;
                self.panel = TuiPanel::Logs;
            }
            KeyCode::Char('n') if self.panel == TuiPanel::Logs => {
                self.context_menu = None;
                self.shift_log_container(1);
            }
            KeyCode::Char('p') if self.panel == TuiPanel::Logs => {
                self.context_menu = None;
                self.shift_log_container(-1);
            }
            KeyCode::Char('m') => {
                self.context_menu = None;
                self.panel = TuiPanel::Resources;
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
        let Some(client) = self.client.as_ref() else {
            self.execution_prompt = None;
            self.status = "Docker client is not available in this TUI session.".to_string();
            return Ok(());
        };
        let result = client.execute_plan(&plan, false).await?;
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
        let Some(client) = self.client.as_ref() else {
            self.status = "Docker client is not available in this TUI session.".to_string();
            return Ok(());
        };
        self.snapshot = client.snapshot().await?;
        self.last_refresh = Instant::now();
        self.rebuild_filtered();
        if self.panel == TuiPanel::Resources {
            let project = self.current_project().map(|project| project.name.clone());
            self.resource_data = project.as_deref().and_then(|project| {
                mark_resource_refresh_pending(self.resource_data.take(), project)
            });
            self.last_resource_refresh = None;
        }
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
            self.list_state
                .select(Some(index.min(self.filtered.len() - 1)));
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
            self.selected = self
                .filtered
                .iter()
                .map(|project| project.name.clone())
                .collect();
        }
    }

    fn current_project(&self) -> Option<&Project> {
        self.list_state
            .selected()
            .and_then(|index| self.filtered.get(index))
    }

    fn shift_log_container(&mut self, delta: isize) {
        let len = self
            .current_project()
            .map(|project| project.containers.len())
            .unwrap_or(0);
        if len == 0 {
            self.log_container_index = 0;
            return;
        }
        self.log_container_index = if delta.is_negative() {
            self.log_container_index
                .saturating_sub(delta.unsigned_abs())
        } else {
            (self.log_container_index + delta as usize).min(len - 1)
        };
        self.status = format!(
            "Log Lens 容器 {}/{}，使用 / 更新关键字过滤。",
            self.log_container_index + 1,
            len
        );
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
            theme: self.theme,
            panel: self.panel,
            status: self.status.clone(),
            context_menu: self.context_menu.clone(),
            execution_prompt: self.execution_prompt.clone(),
            resource_data: self.resource_data.clone(),
            resource_trend: current_resource_trend(
                self.resource_previous.as_ref(),
                self.resource_data.as_ref(),
            ),
            log_container_index: self.log_container_index,
        };
        state.table_state.select(self.list_state.selected());
        state
    }

    fn poll_resource_task(&mut self) -> bool {
        let Some(task) = self.resource_task.as_mut() else {
            return false;
        };
        if !task.is_finished() {
            return false;
        }
        let task = self.resource_task.take().expect("finished resource task");
        match task.now_or_never() {
            Some(Ok(data)) => {
                self.last_resource_refresh = Some(Instant::now());
                if self
                    .resource_data
                    .as_ref()
                    .is_some_and(|previous| previous.project == data.project && !previous.loading)
                {
                    self.resource_previous = self.resource_data.clone();
                } else {
                    self.resource_previous = None;
                }
                self.resource_data = Some(data);
            }
            Some(Err(err)) => {
                let project = self
                    .current_project()
                    .map(|project| project.name.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                self.last_resource_refresh = Some(Instant::now());
                self.resource_previous = self
                    .resource_data
                    .as_ref()
                    .filter(|previous| previous.project == project && !previous.loading)
                    .cloned();
                self.resource_data = Some(ResourcePanelData::sampled(
                    project,
                    crate::audit::now_unix_millis(),
                    vec![ResourceRow::error(
                        "task",
                        "resource sampler",
                        "ERR",
                        err.to_string(),
                    )],
                ));
            }
            None => {}
        }
        true
    }

    fn ensure_resource_sampling(&mut self) -> bool {
        if self.panel != TuiPanel::Resources {
            if let Some(task) = self.resource_task.take() {
                task.abort();
                return true;
            }
            return false;
        }
        if self.resource_task.is_some() {
            return false;
        }
        let Some(project) = self.current_project().cloned() else {
            let changed = self.resource_data.is_some();
            self.resource_data = None;
            self.resource_previous = None;
            return changed;
        };
        let current_project = project.name.clone();
        let needs_project_sample = self
            .resource_data
            .as_ref()
            .map(|data| data.project != current_project)
            .unwrap_or(true);
        let due = self
            .last_resource_refresh
            .map(|instant| instant.elapsed() >= self.resource_refresh_interval)
            .unwrap_or(true);
        if !needs_project_sample && !due {
            return false;
        }

        self.resource_data =
            mark_resource_refresh_pending(self.resource_data.take(), &current_project);
        if needs_project_sample {
            self.resource_previous = None;
        }

        let Some(client) = self.client.clone() else {
            self.resource_data = Some(ResourcePanelData::sampled(
                current_project,
                crate::audit::now_unix_millis(),
                vec![ResourceRow::error(
                    "client",
                    "resource sampler",
                    "ERR",
                    "Docker client is not available",
                )],
            ));
            self.last_resource_refresh = Some(Instant::now());
            return true;
        };

        self.resource_task = Some(tokio::spawn(async move {
            client.project_resources_once(&project).await
        }));
        true
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let mut state = self.dashboard_state();
        render_dashboard(frame, &mut state);
    }
}

pub fn render_dashboard(frame: &mut ratatui::Frame, state: &mut DashboardState) {
    let area = frame.area();
    if is_compact_terminal(area) {
        render_compact_notice(frame, area);
        return;
    }

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

    let left_width = if area.width < 110 { 42 } else { 48 };
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_width),
            Constraint::Percentage(100 - left_width),
        ])
        .split(outer[2]);
    render_projects_table(frame, main[0], state);
    render_ops_deck(frame, main[1], state);
    render_command_bar(frame, outer[3], state);
    render_context_menu(frame, area, state);
}

fn is_compact_terminal(area: Rect) -> bool {
    area.width < MIN_TUI_WIDTH || area.height < MIN_TUI_HEIGHT
}

fn is_compact_terminal_size(terminal_size: (u16, u16)) -> bool {
    terminal_size.0 < MIN_TUI_WIDTH || terminal_size.1 < MIN_TUI_HEIGHT
}

fn render_compact_notice(frame: &mut ratatui::Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(
            "Terminal too small",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!(
            "minimum: {}x{} | current: {}x{}",
            MIN_TUI_WIDTH, MIN_TUI_HEIGHT, area.width, area.height
        )),
        Line::from(""),
        Line::from("Resize the terminal or use dockerctl list / doctor / health."),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .title("dockerctl")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Yellow)),
            ),
        area,
    );
}

fn render_header(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, state: &DashboardState) {
    let palette = theme_palette(state.theme);
    let filter = if state.filter.is_empty() {
        "none"
    } else {
        &state.filter
    };
    let live_badge = Span::styled(" LIVE docker socket ", Style::default().fg(palette.muted));
    let title = Line::from(vec![
        Span::styled(
            " OPS COCKPIT ",
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " DOCKERCTL COMMAND CENTER",
            Style::default()
                .fg(palette.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" | ", Style::default().fg(palette.muted)),
        live_badge,
        Span::styled(
            format!(
                " mode:{} selected:{} sort:{:?} filter:{} ",
                if state.running_only { "active" } else { "all" },
                state.selected.len(),
                state.sort_mode,
                filter
            ),
            Style::default().fg(palette.warning),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title)
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.surface))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(palette.primary)),
            ),
        area,
    );
}

#[derive(Debug, Clone, Copy)]
struct ThemePalette {
    primary: Color,
    accent: Color,
    danger: Color,
    warning: Color,
    success: Color,
    muted: Color,
    surface: Color,
    selection: Color,
}

fn theme_palette(theme: ThemeName) -> ThemePalette {
    match theme {
        ThemeName::Cockpit => ThemePalette {
            primary: Color::Rgb(56, 189, 248),
            accent: Color::Rgb(20, 184, 166),
            danger: Color::Rgb(248, 113, 113),
            warning: Color::Rgb(251, 191, 36),
            success: Color::Rgb(74, 222, 128),
            muted: Color::Rgb(100, 116, 139),
            surface: Color::Rgb(15, 23, 42),
            selection: Color::Rgb(30, 41, 59),
        },
        ThemeName::Industrial => ThemePalette {
            primary: Color::Cyan,
            accent: Color::Blue,
            danger: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
            muted: Color::DarkGray,
            surface: Color::Black,
            selection: Color::Rgb(43, 36, 11),
        },
        ThemeName::Signal => ThemePalette {
            primary: Color::Green,
            accent: Color::Cyan,
            danger: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
            muted: Color::DarkGray,
            surface: Color::Black,
            selection: Color::Rgb(18, 45, 22),
        },
        ThemeName::Ocean => ThemePalette {
            primary: Color::Blue,
            accent: Color::Cyan,
            danger: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
            muted: Color::DarkGray,
            surface: Color::Black,
            selection: Color::Rgb(18, 31, 52),
        },
    }
}

fn render_metric_bar(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let palette = theme_palette(state.theme);
    let metrics = dashboard_metrics(state, palette);
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

    for (index, metric) in metrics.into_iter().enumerate() {
        let text = vec![
            Line::from(vec![
                Span::styled(
                    "KPI ",
                    Style::default()
                        .fg(palette.muted)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(metric.label, Style::default().fg(palette.muted)),
            ]),
            Line::from(Span::styled(
                metric.value,
                Style::default()
                    .fg(metric.color)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                metric.hint,
                Style::default().fg(palette.muted),
            )),
        ];
        frame.render_widget(
            Paragraph::new(text)
                .alignment(Alignment::Center)
                .style(Style::default().fg(metric.color).bg(palette.surface))
                .block(
                    Block::default()
                        .title(metric.label)
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(metric.color)),
                ),
            chunks[index],
        );
    }
}

#[derive(Debug, Clone)]
struct MetricTile {
    label: &'static str,
    value: String,
    hint: &'static str,
    color: Color,
}

fn render_projects_table(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &mut DashboardState,
) {
    let palette = theme_palette(state.theme);
    let header = Row::new(vec![
        Cell::from("Sel"),
        Cell::from("State"),
        Cell::from("Project"),
        Cell::from("Kind"),
        Cell::from("Run"),
        Cell::from("Ports"),
        Cell::from("Risk"),
    ])
    .style(
        Style::default()
            .fg(palette.muted)
            .add_modifier(Modifier::BOLD),
    );

    let rows = state.filtered.iter().map(|project| {
        let is_selected = state.selected.contains(&project.name);
        let selected = if is_selected { "[x]" } else { "[ ]" };
        let risk = project_risk(project, palette);
        let selection_style = selected_project_style(palette);
        let row_style = if is_selected {
            Style::default().bg(palette.selection)
        } else {
            Style::default().bg(palette.surface)
        };
        Row::new(vec![
            Cell::from(selected).style(if is_selected {
                selection_style
            } else {
                Style::default().fg(palette.muted)
            }),
            Cell::from(status_pill(project)).style(project_style(project, palette)),
            Cell::from(project.name.clone()).style(if is_selected {
                selection_style
            } else {
                Style::default().fg(Color::White)
            }),
            Cell::from(project_kind_label(project)).style(Style::default().fg(palette.muted)),
            Cell::from(format!("{}/{}", project.active(), project.containers.len()))
                .style(Style::default().fg(palette.accent)),
            Cell::from(project.ports.len().to_string()).style(Style::default().fg(palette.muted)),
            Cell::from(risk.0).style(Style::default().fg(risk.1).add_modifier(Modifier::BOLD)),
        ])
        .style(row_style)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Min(14),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .title("Projects / Risk Radar")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(palette.primary))
            .style(Style::default().bg(palette.surface)),
    )
    .row_highlight_style(
        Style::default()
            .fg(palette.primary)
            .bg(palette.selection)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    frame.render_stateful_widget(table, area, &mut state.table_state);
}

fn status_pill(project: &Project) -> String {
    format!("[{}]", project.state_code())
}

fn project_kind_label(project: &Project) -> &'static str {
    match project.kind {
        crate::domain::ProjectKind::Compose => "compose",
        crate::domain::ProjectKind::Stack => "stack",
        crate::domain::ProjectKind::Standalone => "single",
    }
}

fn render_ops_deck(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    if state.panel == TuiPanel::Inbox {
        render_inbox_panel(frame, area, state);
        return;
    }
    if state.panel == TuiPanel::Resources {
        render_resources_panel(frame, area, state);
        return;
    }
    if state.panel == TuiPanel::Logs {
        render_logs_panel(frame, area, state);
        return;
    }

    let palette = theme_palette(state.theme);
    let title = match state.panel {
        TuiPanel::Inbox => "Ops Deck / Inbox",
        TuiPanel::Detail => "Ops Deck / Detail",
        TuiPanel::Doctor => "Ops Deck / Doctor",
        TuiPanel::Logs => "Ops Deck / Logs",
        TuiPanel::Resources => "Ops Deck / Resources",
        TuiPanel::Plan(OperationAction::Start) => "Ops Deck / Plan Start",
        TuiPanel::Plan(OperationAction::Stop) => "Ops Deck / Plan Stop",
        TuiPanel::Plan(OperationAction::Restart) => "Ops Deck / Plan Restart",
        TuiPanel::Plan(OperationAction::Remove) => "Ops Deck / Plan Remove",
        TuiPanel::Plan(OperationAction::Purge) => "Ops Deck / Plan Purge",
        TuiPanel::Plan(OperationAction::Prune) => "Ops Deck / Plan Prune",
        TuiPanel::Plan(OperationAction::Rescue) => "Ops Deck / Plan Rescue",
        TuiPanel::Help => "Ops Deck / Help",
    };
    let current = state
        .current_project()
        .map(|project| {
            format!(
                "target:{} state:{} active:{} ports:{}",
                project.name,
                project.state_code(),
                project.active(),
                project.ports.len()
            )
        })
        .unwrap_or_else(|| "target:none".to_string());
    let mut content = vec![
        Line::from(vec![
            Span::styled(
                "CONTROL SURFACE",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" | {current}"), Style::default().fg(palette.muted)),
        ]),
        Line::from(""),
    ];
    content.extend(text_lines(panel_text(state)));
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(palette.surface))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(palette.accent))
                    .style(Style::default().bg(palette.surface)),
            ),
        area,
    );
}

fn render_inbox_panel(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let palette = theme_palette(state.theme);
    let inbox = build_ops_inbox(&state.snapshot, state.resource_data.as_ref());
    let critical = inbox
        .items
        .iter()
        .filter(|item| item.severity == InboxSeverity::Critical)
        .count();
    let warnings = inbox
        .items
        .iter()
        .filter(|item| item.severity == InboxSeverity::Warning)
        .count();
    let categories = inbox_categories(&inbox.items);
    let block = Block::default()
        .title("Ops Deck / Ops Inbox")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.accent))
        .style(Style::default().bg(palette.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(inner);

    let header = vec![
        Line::from(vec![
            Span::styled(
                "Ops Inbox",
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" | Critical:{critical} Warning:{warnings}"),
                Style::default().fg(palette.muted),
            ),
            Span::styled(" | b inbox", Style::default().fg(palette.warning)),
        ]),
        Line::from(Span::styled(
            format!("Categories: {categories}"),
            Style::default().fg(palette.muted),
        )),
        Line::from(Span::styled(
            "Prioritized next actions from health, resources, and cleanup signals.",
            Style::default().fg(palette.muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(header)
            .style(Style::default().bg(palette.surface))
            .block(
                Block::default()
                    .title("Signal Summary")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(palette.primary))
                    .style(Style::default().bg(palette.surface)),
            ),
        chunks[0],
    );

    let rows = inbox
        .items
        .iter()
        .take(8)
        .map(|item| inbox_row(item, palette));
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Length(18),
            Constraint::Min(20),
            Constraint::Length(34),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("Level"),
            Cell::from("Category"),
            Cell::from("Project"),
            Cell::from("Signal"),
            Cell::from("Next Action"),
        ])
        .style(
            Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .title("Action Queue")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(palette.primary))
            .style(Style::default().bg(palette.surface)),
    );
    frame.render_widget(table, chunks[1]);

    frame.render_widget(
        Paragraph::new(
            "Enter plan panel to execute; Inbox only recommends safe preflight commands.",
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(palette.muted).bg(palette.surface))
        .block(
            Block::default()
                .title("Safety")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(palette.muted))
                .style(Style::default().bg(palette.surface)),
        ),
        chunks[2],
    );
}

fn inbox_categories(items: &[InboxItem]) -> String {
    let mut categories = BTreeSet::new();
    for item in items {
        categories.insert(item.category.as_str());
    }
    categories.into_iter().collect::<Vec<_>>().join(", ")
}

fn inbox_row(item: &InboxItem, palette: ThemePalette) -> Row<'_> {
    let color = inbox_severity_color(item.severity, palette);
    Row::new(vec![
        Cell::from(inbox_severity_label(item.severity))
            .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Cell::from(item.category.clone()).style(Style::default().fg(color)),
        Cell::from(item.project.as_deref().unwrap_or("global").to_string())
            .style(Style::default().fg(palette.accent)),
        Cell::from(item.title.clone()),
        Cell::from(item.command.clone()).style(Style::default().fg(palette.warning)),
    ])
    .style(Style::default().bg(palette.surface))
}

fn inbox_severity_label(severity: InboxSeverity) -> &'static str {
    match severity {
        InboxSeverity::Critical => "CRIT",
        InboxSeverity::Warning => "WARN",
        InboxSeverity::Info => "INFO",
    }
}

fn inbox_severity_color(severity: InboxSeverity, palette: ThemePalette) -> Color {
    match severity {
        InboxSeverity::Critical => palette.danger,
        InboxSeverity::Warning => palette.warning,
        InboxSeverity::Info => palette.primary,
    }
}

fn text_lines(text: String) -> Vec<Line<'static>> {
    text.lines()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

fn render_logs_panel(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let palette = theme_palette(state.theme);
    let block = Block::default()
        .title("Ops Deck / Logs / Log Lens")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.accent))
        .style(Style::default().bg(palette.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(inner);

    let Some(project) = state.current_project() else {
        frame.render_widget(
            Paragraph::new("Log Lens\nNo project matches current filter.")
                .style(Style::default().fg(palette.muted).bg(palette.surface)),
            inner,
        );
        return;
    };
    let filter = log_filter_label(state);
    let selected_index = state
        .log_container_index
        .min(project.containers.len().saturating_sub(1));
    let selected_number = if project.containers.is_empty() {
        0
    } else {
        selected_index + 1
    };
    let selected_container = project.containers.get(selected_index);
    let target = selected_container
        .map(|container| container.id.as_str())
        .unwrap_or(project.name.as_str());

    let header = vec![
        Line::from(vec![
            Span::styled(
                "Log Lens",
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" | project {}", project.name),
                Style::default().fg(palette.muted),
            ),
            Span::styled(
                format!(
                    " | container {}/{}",
                    selected_number,
                    project.containers.len()
                ),
                Style::default().fg(palette.warning),
            ),
        ]),
        Line::from(vec![
            Span::styled("Keyword Filter: ", Style::default().fg(palette.muted)),
            Span::styled(filter.clone(), Style::default().fg(palette.primary)),
            Span::styled(
                " | n/p switch container",
                Style::default().fg(palette.muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("Highlight: ", Style::default().fg(palette.muted)),
            Span::styled(
                "ERROR",
                Style::default()
                    .fg(palette.danger)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" / "),
            Span::styled(
                "WARN",
                Style::default()
                    .fg(palette.warning)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(header)
            .style(Style::default().bg(palette.surface))
            .block(
                Block::default()
                    .title("Signal")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(palette.primary))
                    .style(Style::default().bg(palette.surface)),
            ),
        chunks[0],
    );

    let rows = project
        .containers
        .iter()
        .enumerate()
        .map(|(index, container)| {
            let selected = index == selected_index;
            Row::new(vec![
                Cell::from(if selected { ">" } else { " " }),
                Cell::from(container.state.state_code()).style(log_state_style(container, palette)),
                Cell::from(container.name.clone()).style(if selected {
                    Style::default()
                        .fg(Color::White)
                        .bg(palette.selection)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.primary)
                }),
                Cell::from(container.status.clone())
                    .style(log_status_style(&container.status, palette)),
            ])
            .style(if selected {
                Style::default().bg(palette.selection)
            } else {
                Style::default().bg(palette.surface)
            })
        });
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(6),
            Constraint::Min(18),
            Constraint::Percentage(45),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from(""),
            Cell::from("State"),
            Cell::from("Selected Container"),
            Cell::from("Status / Preview Signal"),
        ])
        .style(
            Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .title("Containers")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(palette.primary))
            .style(Style::default().bg(palette.surface)),
    );
    frame.render_widget(table, chunks[1]);

    frame.render_widget(
        Paragraph::new(format!(
            "dockerctl logs {target} --tail 200 | filter: {filter} | highlights: error warn panic"
        ))
        .alignment(Alignment::Center)
        .style(Style::default().fg(palette.muted).bg(palette.surface))
        .block(
            Block::default()
                .title("Command / Log Lens")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(palette.muted))
                .style(Style::default().bg(palette.surface)),
        ),
        chunks[2],
    );
}

fn log_filter_label(state: &DashboardState) -> String {
    if state.filter.is_empty() {
        "none".to_string()
    } else {
        state.filter.clone()
    }
}

fn log_state_style(container: &crate::domain::Container, palette: ThemePalette) -> Style {
    match container.state {
        crate::domain::ContainerState::Unhealthy
        | crate::domain::ContainerState::Restarting
        | crate::domain::ContainerState::Dead => Style::default()
            .fg(palette.danger)
            .add_modifier(Modifier::BOLD),
        crate::domain::ContainerState::Paused => Style::default().fg(palette.warning),
        crate::domain::ContainerState::Running => Style::default().fg(palette.success),
        _ => Style::default().fg(palette.muted),
    }
}

fn log_status_style(status: &str, palette: ThemePalette) -> Style {
    let lower = status.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("panic") || lower.contains("unhealthy") {
        Style::default()
            .fg(palette.danger)
            .add_modifier(Modifier::BOLD)
    } else if lower.contains("warn") || lower.contains("restart") {
        Style::default().fg(palette.warning)
    } else {
        Style::default().fg(palette.muted)
    }
}

fn render_resources_panel(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let palette = theme_palette(state.theme);
    let block = Block::default()
        .title("Ops Deck / Resources / Resource Monitor")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.accent))
        .style(Style::default().bg(palette.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(inner);

    render_resource_summary(frame, chunks[0], state);
    render_resource_table(frame, chunks[1], state);
    render_resource_footer(frame, chunks[2], state);
}

fn render_resource_summary(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let project_name = state
        .current_project()
        .map(|project| project.name.as_str())
        .unwrap_or("none");
    let palette = theme_palette(state.theme);

    let block = Block::default()
        .title("KPI Strip / Project Resources")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(palette.primary))
        .style(Style::default().bg(palette.surface));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    let status = resource_status_lines(state, project_name);
    frame.render_widget(Paragraph::new(status), chunks[0]);

    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(chunks[1]);

    let Some(data) = state.resource_data.as_ref().filter(|data| !data.loading) else {
        render_resource_card(
            frame,
            cards[0],
            "CPU",
            "--",
            "waiting",
            palette.muted,
            palette,
        );
        render_resource_card(
            frame,
            cards[1],
            "MEM",
            "--",
            "waiting",
            palette.muted,
            palette,
        );
        render_resource_card(
            frame,
            cards[2],
            "NET",
            "--",
            "rx / tx",
            palette.muted,
            palette,
        );
        render_resource_card(
            frame,
            cards[3],
            "IO",
            "--",
            "read / write",
            palette.muted,
            palette,
        );
        return;
    };
    let trend = state.resource_trend.as_ref();
    let cpu_subtitle = trend
        .map(|trend| format!("trend {:+.1}%", trend.cpu_delta_percent))
        .unwrap_or_else(|| format!("{} containers", data.summary.containers));
    let mem_subtitle = trend
        .map(|trend| format!("trend {}", format_signed_bytes(trend.memory_delta_bytes)))
        .unwrap_or_else(|| {
            format!(
                "{}/{}",
                format_compact_bytes(data.summary.memory_usage_bytes),
                format_compact_bytes(data.summary.memory_limit_bytes)
            )
        });
    let net_subtitle = trend
        .map(|trend| {
            format!(
                "trend {} / {}",
                format_signed_bytes(trend.network_rx_delta_bytes),
                format_signed_bytes(trend.network_tx_delta_bytes)
            )
        })
        .unwrap_or_else(|| "rx / tx".to_string());
    let io_subtitle = trend
        .map(|trend| {
            format!(
                "trend {} / {}",
                format_signed_bytes(trend.block_read_delta_bytes),
                format_signed_bytes(trend.block_write_delta_bytes)
            )
        })
        .unwrap_or_else(|| "read / write".to_string());

    render_resource_card(
        frame,
        cards[0],
        "CPU",
        &format!("{:.1}%", data.summary.cpu_percent),
        &cpu_subtitle,
        resource_cpu_color(data.summary.cpu_percent),
        palette,
    );
    render_resource_card(
        frame,
        cards[1],
        "MEM",
        &format!("{:.1}%", data.summary.memory_percent),
        &mem_subtitle,
        resource_memory_color(data.summary.memory_percent),
        palette,
    );
    render_resource_card(
        frame,
        cards[2],
        "NET",
        &format!(
            "{} / {}",
            format_compact_bytes(data.summary.network_rx_bytes),
            format_compact_bytes(data.summary.network_tx_bytes)
        ),
        &net_subtitle,
        palette.primary,
        palette,
    );
    render_resource_card(
        frame,
        cards[3],
        "IO",
        &format!(
            "{} / {}",
            format_compact_bytes(data.summary.block_read_bytes),
            format_compact_bytes(data.summary.block_write_bytes)
        ),
        &io_subtitle,
        if data.summary.error_count > 0 {
            palette.warning
        } else {
            palette.success
        },
        palette,
    );
}

fn resource_status_lines(state: &DashboardState, project_name: &str) -> Vec<Line<'static>> {
    let palette = theme_palette(state.theme);
    match state.resource_data.as_ref() {
        Some(data) if data.loading => vec![Line::from(vec![
            Span::styled(
                "Resource Monitor",
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" | project {project_name} | ")),
            Span::styled("sampling...", Style::default().fg(palette.warning)),
            Span::styled(" | KPI loading", Style::default().fg(palette.muted)),
        ])],
        Some(data) => {
            let sample_state = if data.stale {
                Span::styled("refreshing", Style::default().fg(palette.warning))
            } else {
                Span::styled("live", Style::default().fg(palette.success))
            };
            let mut spans = vec![
                Span::styled(
                    "Resource Monitor",
                    Style::default()
                        .fg(palette.primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" | {} | ", data.project)),
                sample_state,
                Span::styled(" | KPI sampled", Style::default().fg(palette.muted)),
                Span::raw(format!(" | err {}", data.summary.error_count)),
            ];
            if let Some(trend) = state.resource_trend.as_ref() {
                spans.push(Span::styled(
                    format!(" | trend {:+.1}%", trend.cpu_delta_percent),
                    Style::default().fg(palette.accent),
                ));
            }
            let mut lines = vec![Line::from(spans)];
            if let Some(error) = data.rows.iter().find_map(|row| row.error.as_deref()) {
                lines.push(Line::from(vec![
                    Span::styled("stats error | ", Style::default().fg(palette.danger)),
                    Span::styled(error.to_string(), Style::default().fg(palette.danger)),
                ]));
            }
            if let Some(trend) = state.resource_trend.as_ref() {
                lines.push(Line::from(vec![
                    Span::styled("trend | ", Style::default().fg(palette.accent)),
                    Span::styled(
                        format!(
                            "CPU {:+.1}% MEM {} NET {} / {} IO {} / {}",
                            trend.cpu_delta_percent,
                            format_signed_bytes(trend.memory_delta_bytes),
                            format_signed_bytes(trend.network_rx_delta_bytes),
                            format_signed_bytes(trend.network_tx_delta_bytes),
                            format_signed_bytes(trend.block_read_delta_bytes),
                            format_signed_bytes(trend.block_write_delta_bytes)
                        ),
                        Style::default().fg(palette.accent),
                    ),
                ]));
            }
            lines
        }
        None => vec![Line::from(vec![
            Span::styled(
                "Resource Monitor",
                Style::default()
                    .fg(palette.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(" | project {project_name} | ")),
            Span::styled("waiting", Style::default().fg(palette.muted)),
        ])],
    }
}

fn render_resource_card(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    title: &'static str,
    value: &str,
    subtitle: &str,
    color: Color,
    palette: ThemePalette,
) {
    let lines = vec![
        Line::from(Span::styled(
            format!("KPI {title}"),
            Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            value.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            subtitle.to_string(),
            Style::default().fg(palette.muted),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.surface))
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(color))
                    .style(Style::default().bg(palette.surface)),
            ),
        area,
    );
}

fn render_resource_table(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let palette = theme_palette(state.theme);
    let Some(data) = state.resource_data.as_ref().filter(|data| !data.loading) else {
        frame.render_widget(
            Paragraph::new("sampling...\nNo resource rows yet.")
                .style(Style::default().bg(palette.surface))
                .block(
                    Block::default()
                        .title("Containers / Resource Rows")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(palette.primary))
                        .style(Style::default().bg(palette.surface)),
                ),
            area,
        );
        return;
    };

    if data.rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No active containers in current project.")
                .style(Style::default().fg(palette.muted).bg(palette.surface))
                .block(
                    Block::default()
                        .title("Containers / Resource Rows")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .border_style(Style::default().fg(palette.primary))
                        .style(Style::default().bg(palette.surface)),
                ),
            area,
        );
        return;
    }

    let sorted_rows = sorted_resource_rows(&data.rows);
    let rows = sorted_rows
        .iter()
        .map(|row| resource_table_row(row, palette));
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(15),
            Constraint::Length(15),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("State"),
            Cell::from("Container"),
            Cell::from("CPU"),
            Cell::from("MEM%"),
            Cell::from("NET rx/tx"),
            Cell::from("IO r/w"),
        ])
        .style(
            Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .title("Containers / Resource Rows")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(palette.primary))
            .style(Style::default().bg(palette.surface)),
    );
    frame.render_widget(table, area);
}

fn resource_table_row(row: &ResourceRow, palette: ThemePalette) -> Row<'_> {
    if let Some(error) = row.error.as_deref() {
        return Row::new(vec![
            Cell::from(row.state.clone()).style(Style::default().fg(palette.danger)),
            Cell::from(row.container_name.clone()).style(Style::default().fg(palette.danger)),
            Cell::from("ERR").style(Style::default().fg(palette.danger)),
            Cell::from("stats").style(Style::default().fg(palette.danger)),
            Cell::from(error.to_string()).style(Style::default().fg(palette.danger)),
            Cell::from(""),
        ])
        .style(Style::default().bg(palette.surface));
    }
    Row::new(vec![
        Cell::from(row.state.clone()),
        Cell::from(row.container_name.clone()),
        Cell::from(format!("{:.1}%", row.cpu_percent)).style(resource_cpu_style(row.cpu_percent)),
        Cell::from(format!("{:.1}%", row.memory_percent))
            .style(resource_memory_style(row.memory_percent)),
        Cell::from(format!(
            "{} / {}",
            format_compact_bytes(row.network_rx_bytes),
            format_compact_bytes(row.network_tx_bytes)
        )),
        Cell::from(format!(
            "{} / {}",
            format_compact_bytes(row.block_read_bytes),
            format_compact_bytes(row.block_write_bytes)
        )),
    ])
    .style(Style::default().bg(palette.surface))
}

fn render_resource_footer(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let palette = theme_palette(state.theme);
    let target = state
        .current_project()
        .and_then(|project| project.containers.first())
        .map(|container| container.id.as_str())
        .unwrap_or("<container>");
    let hint = state
        .resource_data
        .as_ref()
        .and_then(|data| resource_pressure_hint(&data.rows))
        .unwrap_or_else(|| "no pressure signal".to_string());
    frame.render_widget(
        Paragraph::new(format!(
            "r refresh | m resources | hotspot: {hint} | stats: dockerctl stats {target} --json"
        ))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .title("Commands / Resource Monitor")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(palette.muted))
                .style(Style::default().bg(palette.surface)),
        ),
        area,
    );
}

fn resource_cpu_style(cpu_percent: f64) -> Style {
    Style::default().fg(resource_cpu_color(cpu_percent))
}

fn resource_memory_style(memory_percent: f64) -> Style {
    Style::default().fg(resource_memory_color(memory_percent))
}

fn resource_cpu_color(cpu_percent: f64) -> Color {
    if cpu_percent >= 80.0 {
        Color::Red
    } else if cpu_percent >= 50.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn resource_memory_color(memory_percent: f64) -> Color {
    if memory_percent >= 90.0 {
        Color::Red
    } else if memory_percent >= 70.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn format_compact_bytes(bytes: u64) -> String {
    format_bytes(bytes).replace(' ', "")
}

fn render_command_bar(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    state: &DashboardState,
) {
    let palette = theme_palette(state.theme);
    let text = if state.status.is_empty() {
        " mouse: click row select, right-click manage, wheel move | keys: j/k move | space select | / filter | i detail | d doctor | l logs | m resources | 1-5 plan | Enter execute | q quit "
            .to_string()
    } else {
        state.status.clone()
    };
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.primary).bg(palette.surface))
            .block(
                Block::default()
                    .title("Command Bar / Fast Ops")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(palette.muted))
                    .style(Style::default().bg(palette.surface)),
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
    let palette = theme_palette(state.theme);
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
                    format!("{} {:<8}", if selected { ">" } else { " " }, item.label()),
                    context_menu_item_style(*item, selected, palette),
                ),
                Span::styled(
                    item.description(),
                    context_menu_description_style(selected, palette),
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
                .border_style(Style::default().fg(palette.primary))
                .style(Style::default().bg(palette.surface)),
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
    if is_compact_terminal_size(terminal_size) {
        return None;
    }

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

fn context_menu_item_style(item: ContextMenuItem, selected: bool, palette: ThemePalette) -> Style {
    if selected {
        return Style::default()
            .fg(Color::White)
            .bg(palette.primary)
            .add_modifier(Modifier::BOLD);
    }
    match item {
        ContextMenuItem::Remove => Style::default()
            .fg(palette.warning)
            .add_modifier(Modifier::BOLD),
        ContextMenuItem::Purge => Style::default()
            .fg(palette.danger)
            .add_modifier(Modifier::BOLD),
        ContextMenuItem::Rescue => Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::White),
    }
}

fn context_menu_description_style(selected: bool, palette: ThemePalette) -> Style {
    if selected {
        Style::default()
            .fg(Color::White)
            .bg(palette.primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.muted)
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

fn dashboard_metrics(state: &DashboardState, palette: ThemePalette) -> Vec<MetricTile> {
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
    let risk_color = if unhealthy > 0 {
        palette.danger
    } else {
        palette.success
    };
    vec![
        MetricTile {
            label: "Projects",
            value: total.to_string(),
            hint: "fleet size",
            color: palette.primary,
        },
        MetricTile {
            label: "Active",
            value: active.to_string(),
            hint: "running set",
            color: palette.success,
        },
        MetricTile {
            label: "Risk",
            value: unhealthy.to_string(),
            hint: "needs review",
            color: risk_color,
        },
        MetricTile {
            label: "Selected",
            value: selected.to_string(),
            hint: "operation scope",
            color: palette.warning,
        },
        MetricTile {
            label: "Visible",
            value: state.filtered.len().to_string(),
            hint: if state.running_only {
                "active filter"
            } else {
                "all projects"
            },
            color: if state.running_only {
                palette.accent
            } else {
                palette.muted
            },
        },
    ]
}

fn panel_text(state: &DashboardState) -> String {
    match state.panel {
        TuiPanel::Inbox => inbox_text(state),
        TuiPanel::Detail => detail_text(state),
        TuiPanel::Doctor => doctor_text(&state.snapshot),
        TuiPanel::Logs => logs_text(state),
        TuiPanel::Resources => resources_text(state),
        TuiPanel::Plan(action) => state
            .plan_for(action)
            .map(|plan| format_plan(plan, state.execution_prompt.as_ref()))
            .unwrap_or_else(|err| err.to_string()),
        TuiPanel::Help => help_text(),
    }
}

fn inbox_text(state: &DashboardState) -> String {
    let inbox = build_ops_inbox(&state.snapshot, state.resource_data.as_ref());
    let mut text = String::from("Ops Inbox\n");
    for item in inbox.items.iter().take(8) {
        text.push_str(&format!(
            "[{}] {} | {} | {}\n  {}\n",
            inbox_severity_label(item.severity),
            item.category,
            item.project.as_deref().unwrap_or("global"),
            item.title,
            item.command
        ));
    }
    text
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

fn logs_text(state: &DashboardState) -> String {
    let Some(project) = state.current_project() else {
        return "Log Lens\nNo project matches current filter.".to_string();
    };
    let target = project
        .containers
        .first()
        .map(|container| container.id.as_str())
        .unwrap_or(project.name.as_str());
    let filter = if state.filter.is_empty() {
        "none"
    } else {
        state.filter.as_str()
    };
    let mut text = format!(
        "Log Lens\n\
         project: {}\n\
         containers: {}\n\
         tail: 200 lines\n\
         filter: {filter}\n\
         mode: container-switch + keyword-highlight\n\n\
         controls\n\
         l: open this panel\n\
         /: update shared filter\n\
         n/p: switch container\n\
         error/warn: highlighted in output plan\n\n\
         commands\n\
         dockerctl logs {target} --tail 200\n",
        project.name,
        project.containers.len()
    );
    for container in &project.containers {
        text.push_str(&format!(
            "- {} [{}] {}\n",
            container.name,
            container.state.state_code(),
            container.status
        ));
    }
    text
}

fn resources_text(state: &DashboardState) -> String {
    let Some(project) = state.current_project() else {
        return "Resource Monitor\nNo project matches current filter.".to_string();
    };
    let target = project
        .containers
        .first()
        .map(|container| container.id.as_str())
        .unwrap_or(project.name.as_str());
    format!(
        "Resource Monitor\n\
         project: {}\n\
         containers: {} total / {} active\n\
         CPU: use dockerctl stats for live sample\n\
         MEM: use dockerctl stats for live sample\n\
         NET: {} networks, {} published ports\n\
         IO: {} volumes mounted\n\
         images: {}\n\n\
         commands\n\
         dockerctl stats {target}\n\
         dockerctl stats {target} --json\n\n\
         note: v0.3.0 sorts high CPU/high memory/error rows first and feeds Ops Inbox signals.\n",
        project.name,
        project.containers.len(),
        project.active(),
        project.networks.len(),
        project.ports.len(),
        project.volumes.len(),
        project.images.join(", ")
    )
}

fn project_style(project: &Project, palette: ThemePalette) -> Style {
    if project.unhealthy > 0 {
        Style::default().fg(palette.danger)
    } else if project.restarting > 0 {
        Style::default().fg(palette.warning)
    } else if project.paused > 0 {
        Style::default().fg(palette.accent)
    } else if project.active() > 0 {
        Style::default().fg(palette.success)
    } else {
        Style::default().fg(palette.muted)
    }
}

fn selected_project_style(palette: ThemePalette) -> Style {
    Style::default()
        .fg(palette.warning)
        .add_modifier(Modifier::BOLD)
}

fn project_risk(project: &Project, palette: ThemePalette) -> (&'static str, Color) {
    if project.unhealthy > 0 {
        ("HIGH", palette.danger)
    } else if project.restarting > 0 {
        ("LOOP", palette.warning)
    } else if project.paused > 0 {
        ("PAUSE", palette.accent)
    } else if project.active() > 0 {
        ("LOW", palette.success)
    } else {
        ("IDLE", palette.muted)
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
        "b: inbox；i: 详情；d: doctor；l: logs；m: resources",
        "1/2/3/4/5: start/stop/restart/remove/purge 预演",
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
