<p align="center">
  <img src="apps/desktop-ui/public/assets/xuanyu.png" alt="XuanYu Agent" width="180" />
</p>

<h1 align="center">XuanYu Agent · 玄语</h1>

<p align="center">
  <strong>本地优先的桌面 AI 智能体工作台</strong><br/>
  <sub>推理与编排跑在独立 Rust 引擎里，记忆与数据留在你自己的机器上</sub>
</p>

<p align="center">
  <a href="./README.md"><img alt="简体中文" src="https://img.shields.io/badge/简体中文-d9d9d9"></a>
  <a href="./i18n/README.en.md"><img alt="English" src="https://img.shields.io/badge/English-d9d9d9"></a>
  <a href="./i18n/README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/日本語-d9d9d9"></a>
  <a href="./i18n/README.es.md"><img alt="Español" src="https://img.shields.io/badge/Español-d9d9d9"></a>
  <a href="./i18n/README.fr.md"><img alt="Français" src="https://img.shields.io/badge/Français-d9d9d9"></a>
</p>

<p align="center">
  <a href="https://img.shields.io/badge/platform-Windows%2010%2B-blue"><img src="https://img.shields.io/badge/platform-Windows%2010%2B-blue" alt="Platform"/></a>
  <a href="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9"><img src="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9" alt="Version"/></a>
  <a href="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6"><img src="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6" alt="Tech Stack"/></a>
  <a href="https://img.shields.io/badge/engine-15%20crates-16a34a"><img src="https://img.shields.io/badge/engine-15%20crates-16a34a" alt="Cargo Workspace"/></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License"/></a>
</p>

---

**玄语（XuanYu Agent）** 是一款**本地优先**的桌面 AI 智能体工作台。核心推理、工具执行与安全治理运行在独立的原生 Rust 引擎进程中，Electron 只承担界面与设备交互。会话、长期记忆与运行记录全部落在**你自己的 PostgreSQL**（内置便携库或现有库）里——离线可用、数据不出本机，同时可接入任意主流模型与第三方能力。

> [!NOTE]
> 当前为 `0.1.0-preview` 预览版，发布平台为 **Windows 10+**。macOS / Linux 在[路线图](#路线图)中。

## 目录

- [核心特性](#核心特性)
- [架构总览](#架构总览)
- [工作区结构](#工作区结构)
- [核心原理](#核心原理)
- [快速开始](#快速开始)
- [构建发布包](#构建发布包)
- [环境要求](#环境要求)
- [配置说明](#配置说明)
- [安全模型](#安全模型)
- [路线图](#路线图)
- [社区与贡献](#社区与贡献)
- [开源许可](#开源许可)

## 核心特性

- **本地原生引擎** — 推理与编排在独立进程 `xuanyu-engine-server` 中运行，与 UI 完全解耦。引擎崩溃可被壳捕获并自动重启，也可脱离界面单独复用（CLI / 无头服务）。
- **长期记忆** — PostgreSQL + pgvector 语义检索为主，配合全文检索与本地 Tantivy 索引做多路召回，实现跨会话记忆与关键事实沉淀。
- **插件 / MCP / 技能市场** — 应用内浏览并安装插件、MCP 服务与技能，按需扩展 Agent 能力，无需改代码。
- **多模型接入** — 配置向导引导接入主流 Provider，支持按任务（标题 / 规划 / 实现 / 审阅等）路由到不同模型，兼顾成本与效果。
- **多 Agent 协作** — 基于子 Agent 的任务拆解，配合「评审 - 合并」工作流完成复杂工程任务。
- **多渠道接入** — 微信、飞书、钉钉、Telegram、Discord、QQ、Line、WhatsApp 等外部 IM 统一接入同一套会话与记忆。
- **安全治理** — 命令黑名单、git 破坏性指令分级、路径作用域校验、OS 沙箱，高危操作一律需要人工审批。
- **可回退的编辑重发** — 编辑历史中任意一条用户消息并重发时，只精确撤销 AI 本轮产生的文件改动（逐文件内容快照 + 差量回滚），保留你未提交的无关工作。
- **性能细节** — 投机执行隐藏工具往返延迟、上下文自动压缩、截断即继续，长任务不中断。
- **现代桌面体验** — Electron + antd，亮 / 暗主题，内置流式 Markdown、代码高亮与 Diff 视图。

## 架构总览

```text
渲染层 (React / Vite / antd)
        │  IPC / JSON-RPC
        ▼
Rust 引擎 (xuanyu-engine-server)
        │  SQL
        ▼
PostgreSQL + pgvector
```

三条设计主线：

- **进程解耦** — 引擎是独立进程，通过 stdin/stdout 上的 JSON-RPC 与桌面壳双向通信（见 [engine_server.rs](crates/xuanyu-cli/src/bin/engine_server.rs)）。它不依赖界面，可单独运行、单独测试。
- **事件驱动** — 引擎内所有异步变化（Turn 增量、工具进度、权限请求、记忆更新等）统一走 `eventbus`，序列化为 `AppEvent` 主动推送给 UI。UI 不轮询，天然支持多端订阅。
- **存储单一事实源** — 会话、消息、记忆、运行记录、事件日志全部落 PostgreSQL，便携内置库与现有库使用同一套 schema。

## 工作区结构

```text
apps/
  desktop/            Electron 壳（主进程 / preload / 打包配置）
  desktop-ui/         渲染层（React + Vite + antd）
crates/               15 个 Rust crate（Cargo workspace）
proto/                引擎与壳之间的协议定义（app.proto / session.proto）
scripts/              IPC 代码生成、打包与冒烟脚本
```

引擎侧 crate 一览：

| Crate | 职责 |
|-------|------|
| `xuanyu-core` | 引擎集成层：AppCore，会话 / 记忆 / 市场 / 频道 |
| `xuanyu-execution` | 执行运行时：主循环 / 工具循环 / 上下文组装 / token 预算 |
| `xuanyu-session` | 会话域：SessionManager / 线程 / 隔离 / 消息队列 |
| `xuanyu-memory` | 长期记忆：pgvector 语义检索 + FTS + Tantivy + embedding |
| `xuanyu-tools` | 工具运行时：MCP / 远程 MCP / OS 沙箱 / 输出边界 |
| `xuanyu-governance` | 安全治理：命令校验 / git 分级 / 路径校验 / 回滚 / 记账 |
| `xuanyu-channels` | 外部通道：微信 / 飞书 / 钉钉 / Telegram / Discord 等 |
| `xuanyu-eventbus` | 事件总线：spine / router / transformer / backpressure |
| `xuanyu-storage` | 存储与控制面：连接池 / 会话表 / 嵌入式 PostgreSQL |
| `xuanyu-protocol` | 协议契约：Envelope / AppEvent / AppError / DTO |
| `xuanyu-cli` | CLI 与 engine-server（含 `db-external` 辅助命令） |
| `xuanyu-desktop-host` | 桌面宿主桥：暴露可自动转写的 IPC 命令 |
| `xuanyu-agent-bridge` | Agent 桥接：子 Agent 工作区绑定 |
| `xuanyu-agents` | 子 Agent 运行时：watchdog / assess_merge |
| `xuanyu-extensions` | 能力扩展：插件 / MCP / 技能加载 |

## 核心原理

### 一次会话的生命周期

用户消息进入 [AppCore](crates/xuanyu-core/src/lib.rs) 后历经四步：

1. **上下文组装** — [prepare_turn](crates/xuanyu-execution/src/context_assembler.rs) 依据 `loadMode`（lazy / balanced / eager）决定本轮注入哪些内容：历史、记忆检索结果、关键事实等。
2. **主循环** — [run_main_turn_loop](crates/xuanyu-execution/src/main_turn_loop.rs) 在迭代上限内反复调用模型、处理工具调用，支持墙钟死线 `deadline_ms` 与迭代软上限。
3. **工具分发** — 检测原生工具调用或「打印式」工具调用（兼容不原生支持 tool call 的模型），交给 `xuanyu-tools` 执行，结果写回转录进入下一轮。
4. **落定** — 模型不再请求工具时，以 `final_text` 结束本轮。

关键工程细节：

- **上下文压缩** — 预算常量见 [session_context.rs](crates/xuanyu-execution/src/session_context.rs)，由配置 `contextManagement.autoCompactThreshold` 触发，裁剪旧工具结果并追加 handoff summary，避免信息断层。
- **截断即继续** — `finish_reason` 为 `length` / `max_tokens` 时不视为最终答案，注入继续提示进入下一轮。
- **投机执行** — [speculative_tools.rs](crates/xuanyu-core/src/speculative_tools.rs) 在流式产出中参数一旦封口即入队执行，dispatch 时直接取结果，隐藏工具往返延迟。

### 长期记忆

由 [xuanyu-memory](crates/xuanyu-memory/src) 承载，多后端、多召回路线的记忆栈：

- `embedding` 支持三种来源：OpenAI 兼容 API、本地推理、离线哈希（默认，无 API Key 也开箱即用）。
- pgvector 向量检索为主（余弦距离 Top-N）；连续失败 ≥3 次自动熔断，降级为 PostgreSQL FTS 关键词检索。
- 本地 Tantivy 索引（[tantivy_store.rs](crates/xuanyu-memory/src/tantivy_store.rs)）作为彻底离线时的兜底召回。

### 前后端通信

协议契约见 [xuanyu-protocol](crates/xuanyu-protocol/src/lib.rs)：泛型信封 `Envelope<T>` 携带 `correlation_id` / `trace_id` 做请求关联与链路追踪，统一的 `AppError`（含 error code / recoverable / suggestion）取代裸错误码。

前端到引擎的 IPC 桥由脚本自动生成，避免手写造成契约漂移：

| 脚本 | 作用 |
|------|------|
| [gen-desktop-dispatch.py](scripts/gen-desktop-dispatch.py) | 解析 `xuanyu-desktop-host` 函数签名，生成 dispatch 包装 |
| [gen-desktop-ipc-wrappers.py](scripts/gen-desktop-ipc-wrappers.py) | 生成 TypeScript 调用封装 |
| [gen-ipc-command-map.py](scripts/gen-ipc-command-map.py) | 两侧命令覆盖校验 |

改完引擎能力后执行 `pnpm gen:desktop-ipc` 即可对齐前后端契约。

## 快速开始

### 直接使用（推荐）

前往 [Releases](https://github.com/ruo-mu/XuanYu/releases) 下载最新的 `XuanYu Agent Setup <版本>.exe`，双击安装。首次启动会进入配置向导，选择模型即可开始使用。

安装器支持两种数据库方式：

| 方式 | 说明 |
|------|------|
| **内置便携式 PostgreSQL** | 自动下载并初始化（含 pgvector 扩展），**开箱即用**，无需任何额外配置 |
| **连接现有数据库** | 填写连接串并测试连通性（内置 8 秒超时），复用你已有的 PostgreSQL 15+ 实例 |

### 从源码开发

前置：Rust stable（`x86_64-pc-windows-msvc`）、Node.js 20+、pnpm。

```bash
pnpm install
pnpm dev          # 启动 Electron 与本地引擎
```

其他常用命令：

```bash
pnpm typecheck    # 主进程 + 渲染层类型检查
pnpm smoke        # 冒烟测试
pnpm gen:desktop-ipc   # 引擎能力变更后重新生成 IPC 契约
```

## 构建发布包

```bash
pnpm dist
```

该命令依次完成：以 release 模式编译引擎 → 构建前端 → 打包 NSIS 安装包。产物位于 `apps/desktop/release-dist/`。

> [!IMPORTANT]
> 重新执行 `pnpm install` 后会覆盖 electron-builder 的 NSIS 模板补丁。仓库已配置 `postinstall` 钩子自动重新应用，若安装器异常请先确认补丁是否生效。

## 环境要求

| 类别 | 要求 |
|------|------|
| 操作系统 | Windows 10 及以上（当前发布平台） |
| 运行时 | PostgreSQL 15+（仅「连接现有库」时需要；便携模式自动安装） |
| 开发 | Node.js 20+、pnpm 10+、Rust stable + MSVC target |

## 配置说明

运行配置与凭据存放于 `C:\Users\<用户名>\.xuanyu\xuanyu.json`。

配置向导可通过两个入口进入：标题栏 **「文件」菜单**，或系统设置页的 **「配置向导」按钮**——首次配置之后仍可随时重进。写回配置时**合并而非覆盖**既有连接串，避免二次配置丢失数据库连接。配置校验逻辑见 [config_schema.rs](crates/xuanyu-core/src/config_schema.rs)。

## 安全模型

- **命令黑名单** — 拦截 `rm -rf /`、`sudo rm`、`chmod 777`、`eval` 等危险命令（[command_validator.rs](crates/xuanyu-governance/src/command_validator.rs)）。
- **git 分级** — 破坏性指令（`rebase` / `reset` / `clean` / `filter-*` 等）一律拒绝执行（[git_command_policy.rs](crates/xuanyu-governance/src/git_command_policy.rs)）。
- **路径校验** — 文件操作限制在会话隔离作用域内（[path_validator.rs](crates/xuanyu-governance/src/path_validator.rs)）。
- **OS 沙箱** — macOS `sandbox-exec`、Linux `bwrap` 包裹命令（[os_sandbox.rs](crates/xuanyu-tools/src/os_sandbox.rs)）。
- **人工审批** — 高危工具调用触发 `PermissionRequested`，UI 弹窗审批后经 `PermissionAnswered` 回流，全程记账（[turn_ledger.rs](crates/xuanyu-governance/src/turn_ledger.rs)）。
- **回滚安全** — 工作区回退采用内容快照差量还原，**绝不使用** `git reset --hard` 等破坏性命令；被用户手动改过的文件会跳过还原并列入冲突清单。

安全问题的上报方式请见 [SECURITY.md](./.github/SECURITY.md)。

## 路线图

- [ ] 多平台支持（macOS / Linux）
- [ ] 实时会话与事件驱动订阅
- [ ] Agent / 技能市场在线分发与自动更新
- [ ] 更丰富的审阅与权限策略模板
- [ ] 冷启动与性能优化
- [ ] 记忆可视化与混合检索重排序

## 社区与贡献

- [CONTRIBUTING.md](./.github/CONTRIBUTING.md) — 参与开发的规范与流程
- [CODE_OF_CONDUCT.md](./.github/CODE_OF_CONDUCT.md) — 社区行为准则
- [SECURITY.md](./.github/SECURITY.md) — 安全问题上报方式
- [Issue 模板](./.github/ISSUE_TEMPLATE) — Bug 反馈与功能建议

发现 Bug 或有改进建议，欢迎提交 Issue 或 Pull Request。

## 开源许可

本项目采用 [MIT License](./LICENSE)。