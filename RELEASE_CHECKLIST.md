# Release Checklist

发布前按顺序检查：

1. 替换仓库路径

```bash
rg "badwichell007/dockerctl"
```

确认 README、`Cargo.toml` 和 `scripts/install.sh` 中的仓库路径都指向当前实际仓库 `badwichell007/dockerctl`。

2. 准备截图

```bash
mkdir -p assets
```

运行 `hugdocker` 后截一张 TUI 图，保存为：

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
bash -n scripts/open-menu.sh
```

5. 真实 Docker 环境验收

```bash
hugdocker
hugdocker list
hugdocker running
hugdocker doctor
hugdocker health
hugdocker plan remove <project>
hugdocker logs <container> --tail 100
hugdocker stats <container>
```

6. 发布 tag

```bash
git tag v0.4.2
git push origin v0.4.2
```

GitHub Actions 会构建 Linux x86_64/aarch64 release 包。

7. 验证一行安装

```bash
curl -fsSL https://raw.githubusercontent.com/badwichell007/dockerctl/main/scripts/install.sh | bash
hugdocker --help
```
