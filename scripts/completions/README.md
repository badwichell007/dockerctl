# Shell Completions

生成补全脚本：

```bash
dockerctl completion bash > dockerctl.bash
dockerctl completion zsh > _dockerctl
dockerctl completion fish > dockerctl.fish
```

常见安装位置：

```bash
mkdir -p ~/.local/share/bash-completion/completions
dockerctl completion bash > ~/.local/share/bash-completion/completions/dockerctl

mkdir -p ~/.zfunc
dockerctl completion zsh > ~/.zfunc/_dockerctl

mkdir -p ~/.config/fish/completions
dockerctl completion fish > ~/.config/fish/completions/dockerctl.fish
```
