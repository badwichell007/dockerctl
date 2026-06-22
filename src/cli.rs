use std::io;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};

use crate::config::{config_path, load_config, timeline_log_path, write_default_config};
use crate::docker::{docker_cli_available, DockerClient};
use crate::domain::{OperationAction, SortMode};
use crate::health::{analyze_snapshot, global_findings, HealthReport};
use crate::ops::OperationPlanner;
use crate::output::{print_json, print_projects};
use crate::tui;
use crate::{msg, AppResult};

#[derive(Debug, Parser)]
#[command(name = "dockerctl", version, about = "Linux 日常 Docker 项目管理 TUI/CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// 进入全屏 TUI
    Tui,
    /// 兼容旧版经典菜单，当前等价于 TUI
    Menu,
    /// 列出项目摘要
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, value_enum, default_value_t = SortArg::Severity)]
        sort: SortArg,
    },
    /// 仅列出活动项目
    Running {
        #[arg(long)]
        json: bool,
    },
    /// 查看项目详情
    Inspect {
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// 查看容器日志
    Logs {
        container: String,
        #[arg(long, default_value_t = 200)]
        tail: usize,
    },
    /// 显示容器单次资源采样
    Stats {
        container: String,
        #[arg(long)]
        json: bool,
    },
    /// 显示项目健康诊断
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// 显示运行环境健康状态
    Health {
        #[arg(long)]
        json: bool,
    },
    /// 生成操作风险预演
    Plan {
        action: ActionArg,
        projects: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// 启动项目
    Start(OperationArgs),
    /// 停止项目
    Stop(OperationArgs),
    /// 重启项目
    Restart(OperationArgs),
    /// 快速恢复异常项目
    Rescue(OperationArgs),
    /// 删除项目，保留卷和镜像
    Remove(OperationArgs),
    /// 完全删除项目，包含卷和镜像
    Purge(OperationArgs),
    /// 安全清理建议
    SafePrune {
        #[arg(long)]
        json: bool,
    },
    /// prune 的兼容别名，默认只输出安全清理建议
    Prune {
        #[arg(long)]
        json: bool,
    },
    /// 显示事件时间线文件
    Timeline {
        #[arg(long, default_value_t = 50)]
        tail: usize,
        #[arg(long)]
        watch: bool,
    },
    /// 按 profile/group 展示项目
    Profiles {
        #[arg(long)]
        json: bool,
    },
    /// 输出内置日常运维 recipes
    Recipes {
        #[arg(long)]
        json: bool,
    },
    /// watch 模式，周期刷新项目列表
    Watch {
        #[arg(long, default_value_t = 2)]
        interval: u64,
        #[arg(long)]
        running: bool,
        #[arg(long)]
        once: bool,
    },
    /// 生成 shell completion
    Completion { shell: Shell },
    /// 写入默认配置示例
    InitConfig {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Parser)]
pub struct OperationArgs {
    pub projects: Vec<String>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SortArg {
    Severity,
    Name,
    Active,
}

impl From<SortArg> for SortMode {
    fn from(value: SortArg) -> Self {
        match value {
            SortArg::Severity => SortMode::Severity,
            SortArg::Name => SortMode::NameAsc,
            SortArg::Active => SortMode::ActiveDesc,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ActionArg {
    Start,
    Stop,
    Restart,
    Remove,
    Purge,
    Prune,
    Rescue,
}

impl From<ActionArg> for OperationAction {
    fn from(value: ActionArg) -> Self {
        match value {
            ActionArg::Start => OperationAction::Start,
            ActionArg::Stop => OperationAction::Stop,
            ActionArg::Restart => OperationAction::Restart,
            ActionArg::Remove => OperationAction::Remove,
            ActionArg::Purge => OperationAction::Purge,
            ActionArg::Prune => OperationAction::Prune,
            ActionArg::Rescue => OperationAction::Rescue,
        }
    }
}

impl FromStr for ActionArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "start" => Ok(Self::Start),
            "stop" => Ok(Self::Stop),
            "restart" => Ok(Self::Restart),
            "remove" => Ok(Self::Remove),
            "purge" => Ok(Self::Purge),
            "prune" | "safe-prune" => Ok(Self::Prune),
            "rescue" => Ok(Self::Rescue),
            _ => Err(format!("未知动作: {value}")),
        }
    }
}

pub async fn run() -> AppResult<()> {
    let cli = Cli::parse();
    let config = load_config();

    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Tui | Commands::Menu => {
            let client = DockerClient::connect(config)?;
            tui::run(client).await
        }
        Commands::List { json, sort } => {
            let client = DockerClient::connect(config)?;
            let snapshot = client.snapshot().await?;
            let projects = snapshot.projects_sorted(sort.into());
            if json {
                print_json(&projects)
            } else {
                print_projects(&projects);
                Ok(())
            }
        }
        Commands::Running { json } => {
            let client = DockerClient::connect(config)?;
            let snapshot = client.snapshot().await?;
            let projects = snapshot
                .projects_sorted(SortMode::Severity)
                .into_iter()
                .filter(|project| project.active() > 0)
                .collect::<Vec<_>>();
            if json {
                print_json(&projects)
            } else {
                print_projects(&projects);
                Ok(())
            }
        }
        Commands::Inspect { project, json } => {
            let client = DockerClient::connect(config)?;
            let snapshot = client.snapshot().await?;
            let Some(project) = snapshot.project(&project) else {
                return msg("未找到项目。");
            };
            if json {
                print_json(project)
            } else {
                print_projects(std::slice::from_ref(project));
                for container in &project.containers {
                    println!("  - {} [{}] {}", container.name, container.state.state_code(), container.image);
                }
                Ok(())
            }
        }
        Commands::Logs { container, tail } => {
            let client = DockerClient::connect(config)?;
            for line in client.container_logs(&container, tail).await? {
                print!("{line}");
            }
            Ok(())
        }
        Commands::Stats { container, json } => {
            let client = DockerClient::connect(config)?;
            let stats = client.container_stats_once(&container).await?;
            if json {
                print_json(&stats)
            } else {
                println!(
                    "{} mem {}/{} cpu_total {}",
                    stats.name, stats.memory_usage, stats.memory_limit, stats.cpu_total
                );
                Ok(())
            }
        }
        Commands::Doctor { json } => doctor_command(config, json).await,
        Commands::Health { json } => health_command(config, json).await,
        Commands::Plan {
            action,
            projects,
            json,
        } => {
            let client = DockerClient::connect(config)?;
            let snapshot = client.snapshot().await?;
            let plan = OperationPlanner::new(&snapshot).plan(action.into(), &projects)?;
            if json {
                print_json(&plan)
            } else {
                print_plan(&plan);
                Ok(())
            }
        }
        Commands::Start(args) => operation_command(config, OperationAction::Start, args).await,
        Commands::Stop(args) => operation_command(config, OperationAction::Stop, args).await,
        Commands::Restart(args) => operation_command(config, OperationAction::Restart, args).await,
        Commands::Rescue(args) => operation_command(config, OperationAction::Rescue, args).await,
        Commands::Remove(args) => operation_command(config, OperationAction::Remove, args).await,
        Commands::Purge(args) => operation_command(config, OperationAction::Purge, args).await,
        Commands::SafePrune { json } | Commands::Prune { json } => {
            let client = DockerClient::connect(config)?;
            let snapshot = client.snapshot().await?;
            let plan = OperationPlanner::new(&snapshot).plan(OperationAction::Prune, &[])?;
            if json {
                print_json(&plan)
            } else {
                print_plan(&plan);
                Ok(())
            }
        }
        Commands::Timeline { tail, watch } => {
            if watch {
                let client = DockerClient::connect(config)?;
                client.watch_timeline().await
            } else {
                timeline_command(tail)
            }
        }
        Commands::Profiles { json } => profiles_command(config, json).await,
        Commands::Recipes { json } => recipes_command(json),
        Commands::Watch {
            interval,
            running,
            once,
        } => watch_command(config, interval, running, once).await,
        Commands::Completion { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "dockerctl", &mut io::stdout());
            Ok(())
        }
        Commands::InitConfig { force } => {
            let Some(path) = config_path() else {
                return msg("无法确定配置路径。");
            };
            if path.exists() && !force {
                return msg(format!("配置已存在: {}。使用 --force 覆盖。", path.display()));
            }
            write_default_config(&path)?;
            println!("已写入配置: {}", path.display());
            Ok(())
        }
    }
}

fn recipes_command(json: bool) -> AppResult<()> {
    let recipes = vec![
        serde_json::json!({
            "name": "panic-stop",
            "description": "停止选中项目的活动容器，适合临时释放端口/CPU。",
            "command": "dockerctl stop <project> --dry-run"
        }),
        serde_json::json!({
            "name": "rescue-unhealthy",
            "description": "对 unhealthy/restarting 项目生成恢复计划。",
            "command": "dockerctl rescue <project> --dry-run"
        }),
        serde_json::json!({
            "name": "safe-cleanup",
            "description": "查看可安全清理的悬空资源建议。",
            "command": "dockerctl safe-prune"
        }),
        serde_json::json!({
            "name": "preflight-delete",
            "description": "删除前查看容器/网络/卷/镜像影响。",
            "command": "dockerctl plan purge <project>"
        }),
    ];
    if json {
        print_json(&recipes)
    } else {
        for recipe in recipes {
            println!(
                "{}\n  {}\n  {}\n",
                recipe["name"].as_str().unwrap_or_default(),
                recipe["description"].as_str().unwrap_or_default(),
                recipe["command"].as_str().unwrap_or_default()
            );
        }
        Ok(())
    }
}

fn timeline_command(tail: usize) -> AppResult<()> {
    let path = timeline_log_path().unwrap_or_else(|| PathBuf::from("(unknown)"));
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let lines = content.lines().rev().take(tail).collect::<Vec<_>>();
    for line in lines.into_iter().rev() {
        println!("{line}");
    }
    if content.is_empty() {
        println!("暂无时间线事件: {}", path.display());
    }
    Ok(())
}

async fn profiles_command(config: crate::config::AppConfig, json: bool) -> AppResult<()> {
    let client = DockerClient::connect(config)?;
    let snapshot = client.snapshot().await?;
    let mut profiles = std::collections::BTreeMap::<String, Vec<String>>::new();
    for project in snapshot.projects_sorted(SortMode::NameAsc) {
        profiles
            .entry(format!("{:?}", project.kind))
            .or_default()
            .push(project.name);
    }
    if json {
        print_json(&profiles)
    } else {
        for (profile, projects) in profiles {
            println!("{profile}: {}", projects.join(", "));
        }
        Ok(())
    }
}

async fn doctor_command(config: crate::config::AppConfig, json: bool) -> AppResult<()> {
    let client = DockerClient::connect(config)?;
    let snapshot = client.snapshot().await?;
    let projects = analyze_snapshot(&snapshot);
    let findings = global_findings(&snapshot);
    if json {
        return print_json(&serde_json::json!({
            "projects": projects,
            "findings": findings,
        }));
    }
    for project in projects {
        println!("{:?} {}", project.status, project.project);
        for finding in project.findings {
            println!("  - {finding}");
        }
    }
    for finding in findings {
        println!("全局: {finding}");
    }
    Ok(())
}

async fn health_command(config: crate::config::AppConfig, json: bool) -> AppResult<()> {
    let docker_cli = docker_cli_available();
    let client = DockerClient::connect(config)?;
    let docker_daemon = client.ping().await.is_ok();
    let snapshot = if docker_daemon {
        client.snapshot().await.unwrap_or_else(|_| crate::domain::DockerSnapshot::empty())
    } else {
        crate::domain::DockerSnapshot::empty()
    };
    let report = HealthReport {
        docker_cli,
        docker_daemon,
        projects: analyze_snapshot(&snapshot),
        findings: global_findings(&snapshot),
    };
    if json {
        print_json(&report)
    } else {
        println!("docker CLI: {}", if report.docker_cli { "OK" } else { "FAIL" });
        println!("docker daemon: {}", if report.docker_daemon { "OK" } else { "FAIL" });
        for finding in report.findings {
            println!("全局: {finding}");
        }
        Ok(())
    }
}

async fn operation_command(
    config: crate::config::AppConfig,
    action: OperationAction,
    args: OperationArgs,
) -> AppResult<()> {
    let client = DockerClient::connect(config)?;
    let snapshot = client.snapshot().await?;
    let plan = OperationPlanner::new(&snapshot).plan(action, &args.projects)?;
    print_plan(&plan);
    if args.dry_run {
        return Ok(());
    }
    if !args.yes {
        require_confirmation(&plan)?;
    }
    let result = client.execute_plan(&plan, false).await?;
    println!(
        "执行完成: 成功 {} 个，失败 {} 个",
        result.success.len(),
        result.failed.len()
    );
    for failed in result.failed {
        eprintln!("失败 {}: {}", failed.target, failed.message);
    }
    Ok(())
}

async fn watch_command(
    config: crate::config::AppConfig,
    interval: u64,
    running: bool,
    once: bool,
) -> AppResult<()> {
    let client = DockerClient::connect(config)?;
    loop {
        let snapshot = client.snapshot().await?;
        let mut projects = snapshot.projects_sorted(SortMode::Severity);
        if running {
            projects.retain(|project| project.active() > 0);
        }
        print!("\x1b[2J\x1b[H");
        print_projects(&projects);
        if once {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval.max(1))).await;
    }
}

fn print_plan(plan: &crate::ops::OperationPlan) {
    println!("{}", plan.summary);
    if !plan.containers.is_empty() {
        println!("容器: {}", plan.containers.join(", "));
    }
    if !plan.networks.is_empty() {
        println!("网络: {}", plan.networks.join(", "));
    }
    if !plan.volumes.is_empty() {
        println!("卷: {}", plan.volumes.join(", "));
    }
    if !plan.images.is_empty() {
        println!("镜像: {}", plan.images.join(", "));
    }
    for warning in &plan.warnings {
        println!("警告: {warning}");
    }
    if let Some(token) = &plan.confirmation_token {
        println!("确认令牌: {token}");
    }
}

fn require_confirmation(plan: &crate::ops::OperationPlan) -> AppResult<()> {
    let Some(token) = &plan.confirmation_token else {
        return Ok(());
    };
    println!("请输入确认令牌继续: {token}");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim() == token {
        Ok(())
    } else {
        msg("确认令牌不匹配，已取消。")
    }
}
