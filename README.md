# dockerctl

`dockerctl` 是一款面向 Linux 日常运维的 Docker TUI/CLI 管理工具。

它不是 Web 控制台，也不是单纯的容器资源监控器。`dockerctl` 以“项目”为中心，把 Compose、Stack 和 standalone 容器聚合成更适合日常操作的视图，帮助你快速查看状态、预演风险、执行批量操作、诊断异常并安全恢复服务。

一句话定位：

```text
Fast project-first Docker TUI/CLI for Linux daily operations.
```

## 特性

- 项目级视图：自动识别 Compose、Stack、Standalone 容器。
- 高性能 TUI：基于 Rust、ratatui、crossterm，单二进制运行。
- Docker API 后端：通过 bollard 直连 Docker Engine，本地 socket 优先。
- 鼠标和键盘双交互：支持右键菜单、菜单项高亮、多选、滚轮、快捷键。
- 风险预演：删除、完全删除、清理等危险动作先展示影响范围。
- 安全执行：TUI 内执行需要二次确认，危险动作必须输入确认令牌。
- 恢复工作流：`rescue` 提供 Recovery Playbook，适合异常项目快速恢复。
- 诊断能力：`doctor` 检查 unhealthy、restarting、paused、端口冲突等问题。
- 安全清理：`safe-prune` 输出清理建议，避免盲目 prune。
- 审计和时间线：记录操作审计和 Docker events 摘要。
- 脚本友好：核心命令支持 JSON 输出，适合 shell、CI 和自动化脚本。

## 安装

### 一行安装

```bash
curl -fsSL https://raw.githubusercontent.com/badwichell007/dockerctl/main/scripts/install.sh | bash
```

安装完成后默认二进制路径为：

```text
~/.local/bin/dockerctl
```

如果命令不可见，把它加入 `PATH`：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### 指定版本

```bash
DOCKERCTL_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/badwichell007/dockerctl/main/scripts/install.sh | bash
```

### 指定安装目录

```bash
DOCKERCTL_INSTALL_DIR="$HOME/bin" bash ./scripts/install.sh
```

### 源码安装

需要 Rust toolchain。

```bash
git clone https://github.com/badwichell007/dockerctl.git
cd dockerctl
cargo build --release
bash ./scripts/install-cli.sh
```

### 卸载

```bash
bash ./scripts/uninstall-cli.sh
```

### Shell 补全

Bash：

```bash
mkdir -p ~/.local/share/bash-completion/completions
dockerctl completion bash > ~/.local/share/bash-completion/completions/dockerctl
```

Zsh：

```bash
mkdir -p ~/.zfunc
dockerctl completion zsh > ~/.zfunc/_dockerctl
```

Fish：

```bash
mkdir -p ~/.config/fish/completions
dockerctl completion fish > ~/.config/fish/completions/dockerctl.fish
```

## 快速开始

启动 TUI：

```bash
dockerctl
```

列出项目：

```bash
dockerctl list
dockerctl list --json
```

查看项目详情：

```bash
dockerctl inspect myapp
dockerctl inspect myapp --json
```

生成风险预演：

```bash
dockerctl plan remove myapp
dockerctl plan purge myapp --json
```

执行日常操作：

```bash
dockerctl start myapp --dry-run
dockerctl stop myapp --yes
dockerctl restart myapp --yes
```

诊断和恢复：

```bash
dockerctl doctor
dockerctl health --json
dockerctl rescue myapp --dry-run
dockerctl rescue myapp --yes
```

日志和指标：

```bash
dockerctl logs <container-id-or-name> --tail 200
dockerctl stats <container-id-or-name>
```

安全清理和时间线：

```bash
dockerctl safe-prune
dockerctl timeline --tail 100
dockerctl timeline --watch
```

## TUI 使用教程

运行：

```bash
dockerctl
```

TUI 采用 Command Center 布局：

- 顶部：项目总数、活动项目、风险项目、已选项目等指标。
- 左侧：项目表格，按风险、名称或活动状态排序。
- 右侧：Ops Deck，显示详情、诊断、风险预演和 Recovery Playbook。
- 底部：命令栏，显示当前状态和快捷操作。

### 鼠标操作

- 左键点击项目行：选择或反选项目。
- 右键点击项目行：打开管理菜单。
- 鼠标移动到菜单项：高亮当前菜单选择。
- 左键点击菜单项：进入对应详情、诊断或操作预演。
- 鼠标滚轮：上下移动项目光标。

选中的项目会显示 `[x]`，项目名变为黄色加粗，并带有暗色背景。即使光标移动到其他项目，已选项目仍保留颜色提示。

右键菜单包含：

```text
Inspect
Doctor
Start
Stop
Restart
Rescue
Remove
Purge
```

菜单打开后，可以用鼠标移动高亮菜单项，也可以用 `j/k` 或 `↑/↓` 移动高亮项，按 `Enter` 选择当前高亮项。

### 键盘快捷键

```text
j/k 或 ↑/↓    移动项目光标
space         选择/反选当前项目
a             全选/反选当前视图
c             清空选择
/             输入过滤关键字
Backspace     删除过滤字符
x             仅显示活动项目
o             切换排序
r             刷新快照
i             详情面板
d             doctor 面板
1             start 预演
2             stop 预演
3             restart 预演
4             remove 预演
5             purge 预演
Enter         在预演面板打开执行确认
h 或 ?        帮助
q 或 Esc      退出或取消当前确认
```

### TUI 内执行

`dockerctl` 的 TUI 执行流程是“预演优先”：

1. 先选择项目。
2. 选择 `Start`、`Stop`、`Restart`、`Rescue`、`Remove` 或 `Purge`。
3. 右侧 Ops Deck 展示影响范围和风险提示。
4. 在预演面板按 `Enter` 打开执行确认。
5. 普通动作再次按 `Enter` 执行。
6. `Remove`、`Purge` 等危险动作必须输入面板显示的确认令牌后再按 `Enter` 执行。

危险操作会显示 `Safety Rail`。鼠标点击不会直接执行删除或完全删除。

## CLI 使用教程

### 查看项目

```bash
dockerctl list
dockerctl running
dockerctl inspect myapp
```

JSON 输出：

```bash
dockerctl list --json
dockerctl running --json
dockerctl inspect myapp --json
```

### 启动、停止、重启

建议先 dry-run：

```bash
dockerctl start myapp --dry-run
dockerctl stop myapp --dry-run
dockerctl restart myapp --dry-run
```

确认后执行：

```bash
dockerctl start myapp --yes
dockerctl stop myapp --yes
dockerctl restart myapp --yes
```

批量操作：

```bash
dockerctl stop app1 app2 app3 --dry-run
dockerctl restart app1 app2 app3 --yes
```

### 删除和完全删除

删除项目但保留卷和镜像：

```bash
dockerctl plan remove myapp
dockerctl remove myapp
```

完全删除项目，包括卷和镜像：

```bash
dockerctl plan purge myapp
dockerctl purge myapp
```

`remove` 和 `purge` 会显示影响范围，并要求输入确认令牌。例如：

```text
确认令牌: DELETE-myapp
```

只有输入匹配令牌后才会执行。

### 诊断和恢复

检查 Docker 环境：

```bash
dockerctl health
dockerctl health --json
```

检查项目异常：

```bash
dockerctl doctor
dockerctl doctor --json
```

恢复异常项目：

```bash
dockerctl rescue myapp --dry-run
dockerctl rescue myapp --yes
```

`rescue` 会优先处理 unhealthy、restarting、active 容器，并生成恢复重启预案。

### 日志、指标和时间线

```bash
dockerctl logs <container-id-or-name> --tail 200
dockerctl stats <container-id-or-name>
dockerctl stats <container-id-or-name> --json
```

查看 Docker events 时间线：

```bash
dockerctl timeline --tail 100
```

持续监听 Docker events 并写入时间线：

```bash
dockerctl timeline --watch
```

默认状态文件：

```text
~/.local/state/dockerctl/audit.log
~/.local/state/dockerctl/timeline.jsonl
```

### Profiles 和 Recipes

按分组查看项目：

```bash
dockerctl profiles
dockerctl profiles --json
```

查看内置日常运维 recipes：

```bash
dockerctl recipes
dockerctl recipes --json
```

## 配置

生成默认配置：

```bash
dockerctl init-config
```

配置路径：

```text
~/.config/dockerctl/config.toml
```

示例：

```toml
[tui]
refresh_ms = 2000
log_tail = 200
default_filter = ""
theme = "industrial"

[safety]
typed_confirmation = true
allow_yes_for_purge = false

[group_exact]
"mcphub" = "devtools"

[group_prefix]
"redis-" = "cache"
"postgres-" = "database"

[group_image_prefix]
"redis:" = "cache"
"postgres:" = "database"
```

配置后，standalone 容器会按容器名或镜像名前缀归到对应项目组。

## JSON 输出

以下命令提供稳定 JSON 输出，字段使用 `snake_case`：

```bash
dockerctl list --json
dockerctl running --json
dockerctl inspect <project> --json
dockerctl doctor --json
dockerctl health --json
dockerctl plan <action> <project...> --json
dockerctl profiles --json
dockerctl recipes --json
```

## 与其他工具的区别

- 相比通用 Docker TUI，`dockerctl` 更强调项目级动作流和风险预演。
- 相比资源监控工具，`dockerctl` 不只看指标，还覆盖诊断、恢复、删除预演和审计。
- 相比 Web 管理平台，`dockerctl` 是本地单二进制，无需 Agent 或 Server。
- 相比纯脚本，`dockerctl` 提供 TUI、JSON 输出、确认令牌和可测试的安全执行路径。

## 开发

```bash
cargo test --all-targets
cargo check --all-targets
cargo build --release
```

格式化：

```bash
rustup component add rustfmt
cargo fmt
cargo fmt --check
```

安装本地构建：

```bash
bash ./scripts/install-cli.sh
```

卸载本地构建：

```bash
bash ./scripts/uninstall-cli.sh
```

## License

MIT
