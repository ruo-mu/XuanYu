## 变更描述

<!-- 说明本次改动做了什么，以及为什么（聚焦“为什么”而不是流水账）。 -->

## 关联 Issue

<!-- 若有：Fixes #123 -->

## 测试

- [ ] 相关 crate 测试通过
- [ ] `cargo clippy --workspace --all-targets` 无警告
- [ ] `pnpm typecheck` 通过
- [ ] 若改动 IPC 签名，已运行 `pnpm gen:desktop-ipc` 并保持 freshness 检查通过

## 检查项

- [ ] 未在源码文件（src/）中加入内联测试
- [ ] 无调试残留 / 无用导入