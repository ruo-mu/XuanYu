<p align="center">
  <img src="../apps/desktop-ui/public/assets/xuanyu.png" alt="XuanYu Agent" width="180" />
</p>

<h1 align="center">XuanYu Agent · 玄语</h1>

<p align="center">
  <strong>A local-first desktop AI agent workspace</strong><br/>
  <sub>Inference and orchestration run in a standalone Rust engine; memory and data stay on your own machine</sub>
</p>

<p align="center">
  <a href="../README.md"><img alt="简体中文" src="https://img.shields.io/badge/简体中文-d9d9d9"></a>
  <a href="./README.en.md"><img alt="English" src="https://img.shields.io/badge/English-d9d9d9"></a>
  <a href="./README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/日本語-d9d9d9"></a>
  <a href="./README.es.md"><img alt="Español" src="https://img.shields.io/badge/Español-d9d9d9"></a>
  <a href="./README.fr.md"><img alt="Français" src="https://img.shields.io/badge/Français-d9d9d9"></a>
</p>

<p align="center">
  <a href="https://img.shields.io/badge/platform-Windows%2010%2B-blue"><img src="https://img.shields.io/badge/platform-Windows%2010%2B-blue" alt="Platform"/></a>
  <a href="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9"><img src="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9" alt="Version"/></a>
  <a href="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6"><img src="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6" alt="Tech Stack"/></a>
  <a href="https://img.shields.io/badge/engine-15%20crates-16a34a"><img src="https://img.shields.io/badge/engine-15%20crates-16a34a" alt="Cargo Workspace"/></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License"/></a>
</p>

---

**XuanYu Agent (玄语)** is a **local-first** desktop AI agent workspace. Core inference, tool execution, and safety governance run in a standalone native Rust engine process, while Electron handles only the interface and device interaction. Sessions, long-term memory, and run records all live in **your own PostgreSQL** (a bundled portable instance or an existing database)—usable offline, with data never leaving your machine, while still connecting to any mainstream model and third-party capability.

> [!NOTE]
> This is the `0.1.0-preview` release, published for **Windows 10+**. macOS / Linux are on the [roadmap](#roadmap).

## Table of Contents

- [Features](#features)
- [Architecture Overview](#architecture-overview)
- [Workspace Layout](#workspace-layout)
- [How It Works](#how-it-works)
- [Quick Start](#quick-start)
- [Building a Release](#building-a-release)
- [Requirements](#requirements)
- [Configuration](#configuration)
- [Security Model](#security-model)
- [Roadmap](#roadmap)
- [Community and Contributions](#community-and-contributions)
- [License](#license)

## Features

- **Local native engine** — inference and orchestration run in a standalone `xuanyu-engine-server` process, fully decoupled from the UI. If the engine crashes, the shell catches it and restarts it automatically, and the engine can also be reused without the interface (CLI / headless service).
- **Long-term memory** — pgvector semantic search as the primary path, combined with full-text search and a local Tantivy index for multi-path recall, enabling cross-session memory and durable key facts.
- **Plugin / MCP / skill marketplace** — browse and install plugins, MCP servers, and skills in-app to extend agent capabilities on demand, with no code changes.
- **Multi-model support** — a setup wizard walks you through connecting mainstream providers, and tasks (title / planning / implementation / review, etc.) can be routed to different models to balance cost and quality.
- **Multi-agent collaboration** — task decomposition through sub-agents, paired with a review-and-merge workflow for complex engineering tasks.
- **Multi-channel access** — WeChat, Feishu, DingTalk, Telegram, Discord, QQ, Line, WhatsApp, and other external IM platforms all feed into the same set of sessions and memory.
- **Security governance** — command blocklists, tiered git safety, path-scope validation, and an OS sandbox; high-risk operations always require human approval.
- **Editable and resendable history** — editing any past user message and resending it precisely reverts only the file changes the AI produced in that turn (per-file content snapshots + delta rollback), leaving your unrelated uncommitted work untouched.
- **Performance details** — speculative execution hides tool round-trip latency, while automatic context compaction and continue-on-truncation keep long tasks running without interruption.
- **Modern desktop experience** — Electron + antd, light / dark themes, with built-in streaming Markdown, syntax highlighting, and diff views.

## Architecture Overview

```text
Renderer (React / Vite / antd)
        │  IPC / JSON-RPC
        ▼
Rust Engine (xuanyu-engine-server)
        │  SQL
        ▼
PostgreSQL + pgvector
```

Three design principles:

- **Process decoupling** — the engine is a separate process that communicates with the desktop shell over JSON-RPC on stdin/stdout (see [engine_server.rs](../crates/xuanyu-cli/src/bin/engine_server.rs)). It has no dependency on the UI and can be run and tested on its own.
- **Event-driven** — every asynchronous change inside the engine (turn deltas, tool progress, permission requests, memory updates, and so on) flows through `eventbus`, is serialized as `AppEvent`, and is pushed to the UI. The UI never polls, which makes multi-client subscriptions natural.
- **Single source of truth for storage** — sessions, messages, memory, run records, and event logs all live in PostgreSQL, and the portable bundled instance shares one schema with existing databases.

## Workspace Layout

```text
apps/
  desktop/            Electron shell (main process / preload / packaging config)
  desktop-ui/         Renderer (React + Vite + antd)
crates/               15 Rust crates (Cargo workspace)
proto/                Protocol definitions between engine and shell (app.proto / session.proto)
scripts/              IPC codegen, packaging, and smoke scripts
```

The engine crates at a glance:

| Crate | Responsibility |
|-------|------|
| `xuanyu-core` | Engine integration layer: AppCore, sessions / memory / marketplace / channels |
| `xuanyu-execution` | Execution runtime: main loop / tool loop / context assembly / token budget |
| `xuanyu-session` | Session domain: SessionManager / threads / isolation / message queue |
| `xuanyu-memory` | Long-term memory: pgvector semantic search + FTS + Tantivy + embedding |
| `xuanyu-tools` | Tool runtime: MCP / remote MCP / OS sandbox / output boundaries |
| `xuanyu-governance` | Security governance: command validation / git tiers / path validation / rollback / ledger |
| `xuanyu-channels` | External channels: WeChat / Feishu / DingTalk / Telegram / Discord, etc. |
| `xuanyu-eventbus` | Event bus: spine / router / transformer / backpressure |
| `xuanyu-storage` | Storage and control plane: connection pool / session tables / embedded PostgreSQL |
| `xuanyu-protocol` | Protocol contracts: Envelope / AppEvent / AppError / DTO |
| `xuanyu-cli` | CLI and engine-server (including the `db-external` helper command) |
| `xuanyu-desktop-host` | Desktop host bridge: exposes IPC commands that are transpiled automatically |
| `xuanyu-agent-bridge` | Agent bridging: sub-agent workspace binding |
| `xuanyu-agents` | Sub-agent runtime: watchdog / assess_merge |
| `xuanyu-extensions` | Capability extensions: plugin / MCP / skill loading |

## How It Works

### Lifecycle of a Session

Once a user message enters [AppCore](../crates/xuanyu-core/src/lib.rs), it goes through four steps:

1. **Context assembly** — [prepare_turn](../crates/xuanyu-execution/src/context_assembler.rs) uses `loadMode` (`lazy` / `balanced` / `eager`) to decide what gets injected this turn: history, memory retrieval results, key facts, and so on.
2. **Main loop** — [run_main_turn_loop](../crates/xuanyu-execution/src/main_turn_loop.rs) repeatedly calls the model and handles tool calls within an iteration cap, supporting a wall-clock deadline `deadline_ms` and a soft iteration limit.
3. **Tool dispatch** — detects native tool calls or "print-style" tool calls (for models that lack native tool-call support), hands them to `xuanyu-tools` for execution, and writes the results back into the transcript for the next round.
4. **Finalization** — once the model stops requesting tools, the turn ends with `final_text`.

Key engineering details:

- **Context compaction** — budget constants live in [session_context.rs](../crates/xuanyu-execution/src/session_context.rs) and are triggered by the `contextManagement.autoCompactThreshold` config; it trims old tool results and appends a handoff summary to avoid information gaps.
- **Continue on truncation** — when `finish_reason` is `length` / `max_tokens`, the output is not treated as the final answer; a continuation prompt is injected and the next round begins.
- **Speculative execution** — [speculative_tools.rs](../crates/xuanyu-core/src/speculative_tools.rs) enqueues a tool for execution as soon as its arguments are sealed in the streaming output, then reads the result directly at dispatch time, hiding tool round-trip latency.

### Long-Term Memory

Carried by [xuanyu-memory](../crates/xuanyu-memory/src), a memory stack with multiple backends and multiple recall paths:

- `embedding` supports three sources: an OpenAI-compatible API, local inference, and offline hashing (the default — works out of the box with no API key).
- pgvector vector search is the primary path (cosine distance Top-N); after 3 consecutive failures it trips a circuit breaker and degrades to PostgreSQL FTS keyword search.
- A local Tantivy index ([tantivy_store.rs](../crates/xuanyu-memory/src/tantivy_store.rs)) serves as the fallback recall path when fully offline.

### Frontend and Engine Communication

The protocol contract lives in [xuanyu-protocol](../crates/xuanyu-protocol/src/lib.rs): the generic envelope `Envelope<T>` carries `correlation_id` / `trace_id` for request correlation and distributed tracing, and a unified `AppError` (with error code / recoverable / suggestion) replaces bare error codes.

The frontend-to-engine IPC bridge is generated by scripts, so hand-written code can't cause contract drift:

| Script | Purpose |
|------|------|
| [gen-desktop-dispatch.py](../scripts/gen-desktop-dispatch.py) | Parses `xuanyu-desktop-host` function signatures and generates dispatch wrappers |
| [gen-desktop-ipc-wrappers.py](../scripts/gen-desktop-ipc-wrappers.py) | Generates TypeScript call wrappers |
| [gen-ipc-command-map.py](../scripts/gen-ipc-command-map.py) | Validates command coverage on both sides |

After changing engine capabilities, run `pnpm gen:desktop-ipc` to realign the frontend and backend contracts.

## Quick Start

### Use the Installer (Recommended)

Go to [Releases](https://github.com/ruo-mu/XuanYu/releases) and download the latest `XuanYu Agent Setup <version>.exe`, then double-click to install. On first launch a setup wizard appears; pick a model and you're ready to go.

The installer supports two database options:

| Option | Description |
|------|------|
| **Bundled portable PostgreSQL** | Downloaded and initialized automatically (including the pgvector extension), **works out of the box**, with no extra configuration |
| **Connect to an existing database** | Enter a connection string and test connectivity (8-second timeout built in) to reuse your existing PostgreSQL 15+ instance |

### Build from Source

Prerequisites: Rust stable (`x86_64-pc-windows-msvc`), Node.js 20+, and pnpm.

```bash
pnpm install
pnpm dev          # Start Electron and the local engine
```

Other common commands:

```bash
pnpm typecheck    # Main process + renderer type checking
pnpm smoke        # Smoke tests
pnpm gen:desktop-ipc   # Regenerate the IPC contract after engine capability changes
```

## Building a Release

```bash
pnpm dist
```

This command runs three steps in order: compile the engine in release mode → build the frontend → package the NSIS installer. Artifacts land in `apps/desktop/release-dist/`.

> [!IMPORTANT]
> Re-running `pnpm install` overwrites the electron-builder NSIS template patch. The repository ships a `postinstall` hook that reapplies it automatically; if the installer misbehaves, first confirm the patch was applied.

## Requirements

| Category | Requirement |
|------|------|
| Operating system | Windows 10 or later (current release platform) |
| Runtime | PostgreSQL 15+ (only needed for "connect to an existing database"; portable mode installs it automatically) |
| Development | Node.js 20+, pnpm 10+, Rust stable + MSVC target |

## Configuration

Runtime configuration and credentials are stored at `C:\Users\<username>\.xuanyu\xuanyu.json`.

The setup wizard can be opened from two entry points: the **File** menu in the title bar, or the **Setup Wizard** button on the system settings page—you can reopen it at any time after the initial setup. When writing configuration back, existing connection strings are **merged rather than overwritten**, so reconfiguring won't lose your database connection. Validation logic lives in [config_schema.rs](../crates/xuanyu-core/src/config_schema.rs).

## Security Model

- **Command blocklist** — blocks dangerous commands such as `rm -rf /`, `sudo rm`, `chmod 777`, and `eval` ([command_validator.rs](../crates/xuanyu-governance/src/command_validator.rs)).
- **Git tiers** — destructive subcommands (`rebase` / `reset` / `clean` / `filter-*`, etc.) are always rejected ([git_command_policy.rs](../crates/xuanyu-governance/src/git_command_policy.rs)).
- **Path validation** — file operations are confined to the session's isolated scope ([path_validator.rs](../crates/xuanyu-governance/src/path_validator.rs)).
- **OS sandbox** — commands are wrapped with macOS `sandbox-exec` or Linux `bwrap` ([os_sandbox.rs](../crates/xuanyu-tools/src/os_sandbox.rs)).
- **Human approval** — high-risk tool calls raise `PermissionRequested`; after approval in the UI a `PermissionAnswered` flows back, and the whole flow is recorded ([turn_ledger.rs](../crates/xuanyu-governance/src/turn_ledger.rs)).
- **Rollback safety** — workspace reverts use content-snapshot delta restoration and **never** destructive commands such as `git reset --hard`; files you edited by hand are skipped and listed as conflicts.

See [SECURITY.md](../.github/SECURITY.md) for how to report security issues.

## Roadmap

- [ ] Multi-platform support (macOS / Linux)
- [ ] Real-time sessions and event-driven subscriptions
- [ ] Online distribution and auto-update for the Agent / skill marketplace
- [ ] Richer review and permission policy templates
- [ ] Cold-start and performance optimization
- [ ] Memory visualization and hybrid-retrieval reranking

## Community and Contributions

- [CONTRIBUTING.md](../.github/CONTRIBUTING.md) — development guidelines and workflow
- [CODE_OF_CONDUCT.md](../.github/CODE_OF_CONDUCT.md) — community code of conduct
- [SECURITY.md](../.github/SECURITY.md) — how to report security issues
- [Issue Templates](../.github/ISSUE_TEMPLATE) — bug reports and feature requests

Found a bug or have an idea for improvement? Issues and pull requests are welcome.

## License

This project is licensed under the [MIT License](../LICENSE).