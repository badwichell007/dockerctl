# Release Checklist

发布前按顺序检查：

1. 替换仓库路径

```bash
rg "badwichell007/dockerctl"
```

确认 README、Cargo.toml 和 `scripts/install.sh` 中的仓库路径为真实 GitHub `owner/repo`。

2. 准备截图

```bash
mkdir -p assets
```

运行 `dockerctl` 后截一张 TUI 图，保存为：

```text
assets/screenshot.png
```

3. 安装 rustfmt 并格式化

```bash
rustup component add rustfmt
cargo fmt
cargo fmt --check
```

4. 运行验证

```bash
cargo test --all-targets
cargo check --all-targets
cargo build --release
bash -n scripts/install.sh
bash -n scripts/install-cli.sh
bash -n scripts/uninstall-cli.sh
```

5. 真实 Docker 环境验收

```bash
dockerctl
dockerctl list
dockerctl running
dockerctl doctor
dockerctl health
dockerctl plan remove <project>
dockerctl logs <container> --tail 100
dockerctl stats <container>
```

6. 发布 tag

```bash
git tag v0.1.0
git push origin v0.1.0
```

GitHub Actions 会构建 Linux x86_64/aarch64 release 包。

7. 验证一行安装

```bash
curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/scripts/install.sh | bash
dockerctl --help
```
