# 参与贡献

感谢你有兴趣为 XuanYu Agent 贡献代码。请阅读本文档，它能让你的贡献更顺畅。

## 代码规范

- **源码与测试分离**：源码文件不包含任何内联测试（禁止 `#[cfg(test)]` 块与 `mod tests`）。测试只放在各 crate 的 `tests/` 目录。
- **Rust Clippy**：提交前请保证 `cargo clippy --workspace --all-targets` 无警告（0 warnings）。
- **前后端类型检查**：`pnpm typecheck`（含 Electron 主进程与渲染层）。
- **IPC 生成**：若改动了命令分发的函数签名，请运行 `pnpm gen:desktop-ipc` 重新生成 wrapper，并保持 `scripts/check-desktop-ipc-fresh.py` 检查通过。

## 工作流

1. Fork 或基于 `master` 新建分支。
2. 提交信息采用简洁的祈使句说明「为什么」改动，而非流水账。
3. 提交前本地验证：相关 crate 测试、clippy、前端 typecheck 通过。
4. 提交可供审查的 Pull Request。

## 提交信息示例

```
Fix marketplace pagination race; drop fake sample catalog
```

建议先从 [README](../README.md) 了解整体架构，再从 Issues 中选择任务入手。