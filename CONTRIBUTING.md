# Contributing

感谢参与 `dockerctl`。本项目面向 Linux 日常 Docker 管理，优先保证可靠、安全和高性能。

## 原则

- KISS：优先简单、可理解、可测试的实现。
- YAGNI：不要为未明确需要的未来能力预留复杂抽象。
- DRY：跨 CLI、TUI、JSON 的逻辑必须复用领域层和操作计划层。
- SOLID：模块保持单一职责，危险操作只通过 `OperationPlan` 执行。

## 本地验证

```bash
cargo fmt --check
cargo test
cargo check
cargo build --release
```

## 安全约束

- 删除、purge、prune、批量停止等危险操作必须先展示影响范围。
- 新增 mutating 命令必须支持 `--dry-run` 或复用现有 `OperationPlan`。
- 不要把真实凭据、私有 registry token 或生产 API 地址写入测试和文档。
