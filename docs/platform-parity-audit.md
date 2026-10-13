# 三端平台一致性审计

审计目标：找出代码中所有「只在 Windows 成立」或「在 macOS / Linux 上会失败或行为不一致」的实现，作为后续改造的清单。

设计前提：本项目从立项起就是三端原生跨平台（Rust 引擎 + Electron 外壳本身没有平台绑定），本清单记录的是「实现尚未铺开到三端」的地方，不包含任何兼容层、回退开关或降级路径的设计。

统计口径：致命 = 三端中至少一端无法启动或核心功能不可用；受损 = 能启动但行为错误或功能缺失；未验证 = 代码里有分支但从未在对应平台编译/运行过。

---

## 一、致命：macOS / Linux 现在跑不起来

### 1.1 内置便携 PostgreSQL 只有 Windows 实现

这是最大的阻塞项。整条「下载 → 解压 → initdb → 装扩展 → 起服务」链路全部只对 Windows 成立。

| 位置 | 现状 |
| --- | --- |
| `crates/xuanyu-storage/src/embedded_postgres.rs:15-39` | 归档名、下载 URL、SHA256、镜像地址全部写死为 win64 版本，且整段包在 `#[cfg(windows)]` 内，非 Windows 没有任何下载源定义 |
| `crates/xuanyu-storage/src/embedded_postgres.rs:214-245` | 非 Windows 分支直接返回 `Err`，要求用户手动放置归档（实际不可用） |
| `crates/xuanyu-storage/src/embedded_postgres.rs:266-295` | 初始化硬编码 `initdb.exe`，参数 `-D ... -U xuanyu --encoding=UTF8 --locale=C`；没有 Unix 下的数据目录权限处理 |
| `crates/xuanyu-storage/src/embedded_postgres.rs:297-345` | pgvector 扩展同样只下载 Windows 二进制 |
| `crates/xuanyu-storage/src/embedded_postgres.rs:732` | 解压调用 `powershell Expand-Archive`，非 Windows 无法执行 |
| `crates/xuanyu-storage/src/embedded_postgres.rs:712-765` | 解压后校验逻辑按 `bin/initdb.exe` 判定安装成功，围绕 Windows 目录结构 |
| `crates/xuanyu-storage/src/embedded_postgres.rs:96-116` | 目录布局与固定连接串 `postgres://xuanyu@127.0.0.1:55432/xuanyu` |

**事实澄清**：官方 `postgresql.org` 的下载页在 mac/linux 上只给「安装器 + 包管理器」入口（EDB 交互式安装器、Homebrew、Postgres.app、各发行版仓库），**没有** Windows 那种免安装 `.zip` 归档。但这**不等于** mac/linux 无法便携：conda-forge 提供 `postgresql` 与 `pgvector` 的可重定位构建（`linux-64`、`osx-arm64`、`osx-64` 均可用），Homebrew 有 `pgvector` 公式，macOS 还有 Postgres.app（一个 `.app` 内即完整 Postgres 服务端）。所以「便携」在三端都成立，获取渠道与随包分发形态的定案见第五节决策 1。

参考：`crates/xuanyu-storage/src/paths.rs:65-90` 里已有 Unix 分支（缓存目录走 `~/.cache/xuanyu/postgres`），说明目录层是跨平台的，缺口在获取与初始化层。

### 1.2 Linux 上 node-pty 不可用（终端功能直接废掉）

- `package.json:32-38`：`pnpm.onlyBuiltDependencies` 只允许 `electron` / `esbuild` / `sharp` 执行构建脚本，**不含 `node-pty`**。
- node-pty 自带的 `prebuilds/` 只有 `win32-x64`、`win32-arm64`、`darwin-arm64`、`darwin-x64`，**没有 Linux 产物**。
- 结果：pnpm 10 默认阻止 `node-pty` 的 install 脚本，Linux 上既不会编译也不会下载到产物，终端面板无法加载原生模块。
- 相关代码：`apps/desktop/src/main/terminal/TerminalManager.ts:86-108`（`pty.spawn`）。

### 1.3 打包配置缺失（mac / linux 只有裸目录）

`apps/desktop/package.json:114-135`：

- `mac.target` 与 `linux.target` 都只有 `["dir"]`，没有 `dmg` / `zip`、`AppImage` / `deb` / `rpm`。
- `mac` 段缺失：`icon`（无 `.icns`）、`category`、`hardenedRuntime`、`entitlements`、`notarize`。
- `linux` 段缺失：`icon`、`category`、`maintainer`、desktop entry 元数据。
- `apps/desktop/build/icons/` 只有 `icon.png` 与 `icon.ico`，**缺 `icon.icns`**。
- `nsis` 段（`package.json:83-101`）与 `build/installer.nsh`、`build/install-license.txt` 是 Windows 安装包专属；mac/linux 侧没有对应的许可展示与安装体验。

`extraResources` 的引擎二进制名是对的（Windows `xuanyu-engine-server.exe`，mac/linux 无扩展名），壳层 `apps/desktop/src/main/engine/EngineRpcClient.ts:151-175` 也已按平台取值，这部分不用动。

### 1.4 脚本层只有 Windows 能跑

| 位置 | 现状 |
| --- | --- |
| `scripts/dev.ps1:9` | 硬编码本机路径 `D:\develop\environment\xuanyu-agent-app-env.ps1` |
| `scripts/build.ps1:4` | 同上，硬编码 `D:\develop\environment\...` |
| `scripts/smoke.ps1` | PowerShell-only；且 `run -p xuanyu-cli` 缺 `--bin xuanyu`（该 crate 有两个二进制，`cargo run` 无法自选），另需外部 PostgreSQL 才能跑通 |
| `scripts/patch-nsis-templates.ps1` + `.mjs` | 内容本身是 NSIS 专属（合理），但入口只服务 Windows |
| `package.json:26-27` | `smoke` / `build:all` 直接调用 `powershell -NoProfile ...` |
| `scripts/gen-*.py`、`scripts/check-desktop-ipc-fresh.py` | Python 实现，本身跨平台，无需改 |

### 1.5 CI 与发布没有 mac / linux

- `.github/workflows/ci.yml:7-32`：Rust 的 check / clippy / test job 固定 `windows-latest`，即使有 16 处平台条件编译，mac/linux 分支也从未被编译过。
- `.github/workflows/release.yml:2-37`：只有 `build-windows`（`windows-latest`，target `x86_64-pc-windows-msvc`），产物只上传 `*.exe`；没有 mac / linux job，也没有 target 矩阵（`x86_64-apple-darwin`、`aarch64-apple-darwin`、`x86_64-unknown-linux-gnu`）。

---

## 二、受损：能跑但行为不对或功能降级

### 2.1 OS 沙箱的平台处理不均衡

`crates/xuanyu-tools/src/os_sandbox.rs`

- `:14-28` `os_sandbox_exec_block_reason()`：只认 macOS 的 `sandbox-exec` 与 Linux 的 `bwrap`；**Windows 被视为「本平台没有原生进程包装器」，开启沙箱后直接 fail-closed 拒绝一切 exec / start_process**。
- `:65` `base_shell_command()` 的 Unix 分支硬编码 `/bin/bash -lc`，既不遵循 `$SHELL`，也不适用于没有 bash 的系统（NixOS、精简容器）。
- `:52-68` Windows 分支走 `powershell.exe`，这是原生写法，本身没问题。
- 结论：Windows 上的沙箱是「无实现」，unix 上的沙箱依赖两个外部可执行文件存在。

### 2.2 进程管理的 Windows-only 标记

- `crates/xuanyu-tools/src/mcp.rs:301-325`：仅 Windows 用 `std::os::windows::process::CommandExt` 设置 creation flags，非 Windows 没有对应实现。
- `crates/xuanyu-agent-bridge/src/worker_session.rs:18-27`：仅 Windows 设 `CREATE_NO_WINDOW`。
- `crates/xuanyu-tools/src/lib.rs:10996`：硬编码 `taskkill` 终止进程。

### 2.3 高风险命令清单偏 Windows，且是子串匹配

`crates/xuanyu-tools/src/lib.rs:10909-10931`：

- 含有 Windows 概念 `del /s`、`format `、`shutdown`，也有跨平台的 `rm -rf`、`mkfs`、`dd if=`、`git push --force` 等。
- 缺少 unix 特有的高危写法（如 `chmod -R 777 /`、重定向写块设备、`mv /*`）。
- 用 `contains` 子串匹配，`format ` 会误伤任何含该词的普通命令。

### 2.4 终端 shell 选择

- `apps/desktop/src/main/terminal/TerminalManager.ts:22-57`：Windows 侧 `cmd.exe` / `pwsh.exe` / `powershell.exe` 候选较完整；Unix 侧取 `process.env.XUANYU_SHELL || process.env.SHELL || '/bin/bash'`——`XUANYU_SHELL` 是本项目自造变量，除让个别用户手动指定解释器外没有意义，应移除，按平台原生取值即可（unix 用 `$SHELL` 兜底 `/bin/sh`）。
- `TerminalShellKind` 在 unix 上被统一标成 `bash`，实际可能是 zsh/fish，属于标签失真。
- `apps/desktop-ui/src/components/terminal/BottomTerminalPanel.tsx:34`：渲染层同样写 `platform === 'win32' ? 'powershell' : 'bash'`，把 Unix 默认固定成 `bash`，与主进程取值策略不一致。
- `TerminalManager.ts:86-108`：`useConpty` 仅 Windows 生效（这是原生正确行为）。

### 2.5 窗口与菜单（基本正确，需回归）

- `apps/desktop/src/main/index.ts:115-136`：Windows 自绘标题栏 + `titleBarOverlay`，macOS 用 `hiddenInset` 与红绿灯位置——属于原生写法。
- `apps/desktop/src/main/index.ts:176-223`：单实例锁、菜单隐藏、`window-all-closed` 按平台分支（macOS 关窗不退进程），符合各平台惯例。
- `apps/desktop/src/main/appMenu.ts:18-47`：使用 `CmdOrCtrl`。
- `apps/desktop-ui/src/components/shell/TitleBarMenus.tsx:33`：darwin 显示 `⌘`，其它显示 `Ctrl`。
- `apps/desktop-ui/src/utils/electronChrome.ts:60-67`：非 win32 跳过自绘 chrome 同步，正确。
- 结论：这一层不用改，但要在三端各跑一次回归确认。

### 2.6 数据目录之外的资产路径

- `apps/desktop/src/preload/index.ts:33-43` 暴露 `platform`，渲染层据此分支——这是现有机制，保留。

### 2.7 数据落位机制只有 Windows 有，且锚点不是平台惯例

- 数据根只由环境变量 `XUANYU_DATA_DIR` 决定（`crates/xuanyu-storage/src/paths.rs:104-109`），而该变量只有 Windows 安装器会注入：`apps/desktop/build/installer.nsh:528-538` 的 `.onInstSuccess` 写 `~\.xuanyu\launch.conf`（`dataDir=<安装目录>\data`），`apps/desktop/src/main/index.ts:32-49` 读出后经 `apps/desktop/src/main/engine/EngineRpcClient.ts:62` 注入。
- 因此 mac / linux **没有任何入口**改变数据位置，只能落 `~/.xuanyu`。
- 「数据放安装目录」这一形态在 mac / linux 物理上不成立：`.app` 是不可写的签名包（写入即破坏签名封印），`AppImage` 是只读挂载，`deb` 装 `/opt` 需 root。Windows 上可行是因为安装器强制了「仅为当前用户安装」（`installer.nsh:111-119`），绕开了 Program Files 的写权限限制。
- `appDataDir` 配置项（`apps/desktop-ui/src/foundation/config/types.ts:348`）只在 `apps/desktop-ui/src/init/ReviewStep.tsx:129` 做展示，Rust 侧从未读取——孤立字段。
- `crates/xuanyu-storage/src/paths.rs:41-55` 的 `resolve_app_dir_from_selection()` 除测试外没有任何调用方——孤立代码。
- 配置锚点 `~/.xuanyu/xuanyu.json`（`paths.rs:21-28`）在三端都不是平台惯例路径。
- 日志目录目前挂在数据根之下（`crates/xuanyu-storage/src/lib.rs:133` 创建 `{数据根}/logs`），没有独立的平台日志目录。

---

## 三、未验证 / 基础设施缺口

1. **缺 `.gitattributes`**：仓库没有行尾约束，文件随检出平台变化（NSIS 补丁脚本已经因此踩过一次坑：LF 模板 vs CRLF here-string 导致补丁静默失效）。
2. **Unix 侧条件编译从未被编译**：全仓库 `cfg(windows)` / `cfg(unix)` / `cfg(target_os)` / `std::os::windows` 共 16 处，分布在 5 个文件（`xuanyu-agent-bridge`、`xuanyu-tools`、`xuanyu-storage`×3），没有 CI 覆盖。
3. **`.cargo/config.toml:12-13`**：`+crt-static` 只作用于 `x86_64-pc-windows-msvc`，对 mac/linux 无影响，无需改动。
4. **签名与公证**：macOS 未签名/未公证会让用户遇到 Gatekeeper 拦截；Windows 侧也无签名（当前靠 NSIS 安装包分发）。本轮不做，见第五节决策 2，落地时需要各自的证书与账号。
5. **内置 PostgreSQL 的 mac / linux 分支从未构建与运行过**：`embedded_postgres.rs` 的非 Windows 路径直接返回 `Err`，conda-forge 归档的可重定位性、pgvector 与 PostgreSQL 的 ABI 匹配、Unix 下数据目录权限，均无实测记录。这是全项目风险最高的一项，必须先在三端各跑通一次再谈打包。
6. **文档口径**：`README.md:21` 平台徽章写 `Windows 10+`；`:33` 说明「发布平台为 Windows 10+，macOS / Linux 在路线图中」；`:196` 前置条件表；`:217-219` 路线图里挂着「多平台支持（macOS / Linux）」。三端落地后这几处都要改。`i18n/README.{en,ja,es,fr}.md` 同步。
7. `README.md:211` 已经写明 OS 沙箱是 macOS `sandbox-exec` / Linux `bwrap`，与 2.1 的结论一致，Windows 侧无对应实现。

---

## 四、清单汇总

| # | 位置 | 影响端 | 问题 | 类别 |
| --- | --- | --- | --- | --- |
| 1 | `embedded_postgres.rs:15-39,214-245` | mac/linux | 下载源与安装流程只有 Windows | 致命 |
| 2 | `embedded_postgres.rs:266-295` | mac/linux | `initdb.exe` 硬编码、无 Unix 权限处理 | 致命 |
| 3 | `embedded_postgres.rs:297-345` | mac/linux | pgvector 只下载 Windows 二进制 | 致命 |
| 4 | `embedded_postgres.rs:732` | mac/linux | `powershell Expand-Archive` 解压 | 致命 |
| 5 | `package.json:32-38` | linux | `node-pty` 未允许构建、无 Linux 预编译产物 | 致命 |
| 6 | `apps/desktop/package.json:114-135` | mac/linux | 只有 `dir` 产物，缺 dmg/AppImage/deb 及元数据 | 致命 |
| 7 | `apps/desktop/build/icons/` | mac | 缺 `icon.icns` | 致命 |
| 8 | `scripts/dev.ps1:9`、`scripts/build.ps1:4` | mac/linux | 硬编码本机路径 + PowerShell-only | 致命 |
| 9 | `package.json:26-27` | mac/linux | `smoke` / `build:all` 绑定 PowerShell | 致命 |
| 10 | `.github/workflows/ci.yml:7-32` | mac/linux | Rust job 只在 windows-latest | 致命 |
| 11 | `.github/workflows/release.yml:2-37` | mac/linux | 无 mac/linux 构建与 target 矩阵 | 致命 |
| 12 | `os_sandbox.rs:14-28` | win | 开启沙箱后 Windows 拒绝所有 exec | 受损 |
| 13 | `os_sandbox.rs:65` | linux/mac | 硬编码 `/bin/bash`，不遵循 `$SHELL` | 受损 |
| 14 | `lib.rs:10996` | mac/linux | 硬编码 `taskkill` | 受损 |
| 15 | `mcp.rs:301-325`、`worker_session.rs:18-27` | mac/linux | 仅 Windows 的进程创建标志 | 受损 |
| 16 | `lib.rs:10909-10931` | 三端 | 黑名单偏 Windows、子串误伤 | 受损 |
| 17 | `BottomTerminalPanel.tsx:34` | mac/linux | 渲染层把默认 shell 固定为 `bash` | 受损 |
| 18 | 仓库根 | 三端 | 缺 `.gitattributes` | 未验证 |
| 19 | 5 个文件的 16 处条件编译 | mac/linux | 从未编译过 | 未验证 |
| 20 | `README.md:21,33,196,217-219` + `i18n/*` | 三端 | 平台口径与路线图 | 未验证 |
| 21 | `scripts/smoke.ps1` | 三端 | 缺 `--bin`、依赖外部数据库 | 未验证 |
| 22 | `paths.rs:21-33,76-91`、`lib.rs:133` | 三端 | 目录布局不遵循平台惯例，日志未独立 | 受损 |
| 23 | `installer.nsh:528-538`、`index.ts:32-49` | mac/linux | 数据落位只由 Windows 安装器驱动，`launch.conf` 是第二套机制 | 受损 |
| 24 | `types.ts:348`、`paths.rs:41-55` | 三端 | `appDataDir` 是孤立字段、`resolve_app_dir_from_selection` 无调用方 | 受损 |

---

## 五、决策点与已定结论

### 逐项结论

1. **【已定】数据库形态：仅内置，用户不接触任何数据库配置**（取代原「双变体」方案）。三端产物一律内置 PostgreSQL + pgvector，装完即用。用户可见的数据库配置入口全部删除：连接串输入、主机/端口/用户名/密码表单、`db-external` 辅助命令、`memory.postgres.connectionString` 的用户写入路径、NSIS 数据库配置页。连接参数由应用固定（回环地址、固定端口、固定库名与角色），配置文件不再暴露可编辑的数据库字段。原「精简版」产物形态与对应运行时分流一并取消。

   **内置 PostgreSQL 来源**（按「官方出品、长期稳定、维护量低」取舍）：

   - **Windows → EDB 官方「binaries only」归档**。产物出自 PostgreSQL 官方 Windows 构建，直接沿用。
   - **macOS / Linux → conda-forge 可重定位构建**（`postgresql`，覆盖 `osx-arm64` / `osx-64` / `linux-64`）。两端共用同一来源，不自建交叉编译链。
   - **不采用系统仓库**（Homebrew / apt / dnf）：要求用户先有包管理器与 root 权限，版本随发行版漂移。
   - **不采用自建三端构建链**：长期维护成本高于收益。

   **pgvector 与 PostgreSQL 同源同版本打进产物**（`vector.control` + `vector.so` / `vector.dll`）。ABI 不匹配会直接加载失败，这是打包期必须逐平台验证的一点。

   **分发方式：预编译便携态随产物内置，运行期不做任何下载或解压**。三端各自的 PostgreSQL 以「已编译、扩展已就位」的便携态进入安装包，安装时只是拷贝文件。「安装时下载数据库」「安装时解压归档」这类步骤整体取消（现有 `portable-install` 链路删除，见第十项）。

   **获取方式：构建期拉取，仓库不存放任何二进制**。构建阶段按平台下载对应来源并剥离后打进产物（Windows 取 EDB 归档，mac / linux 取 conda-forge 包），仓库内只保留获取脚本。这样「装完即可用、安装期零下载」成立，仓库也不会涨到 GB 级。连续同一标签重复构建时按来源 URL 与校验和复用已拉取的结果，不重复下载。

   **程序与数据的落位分离**：PG 程序文件（`bin` / `lib` / `share` / 扩展）随产物落在安装目录，只读；PG 数据目录（`initdb` 产物）落在平台标准数据目录，可写，随应用启停。

   **体积与瘦身**：在构建期剥离 `include/`、`doc/`、`*.lib`、`*.pdb`、`symbols/`、`pgAdmin`、`StackBuilder`，以及运行期不用的客户端工具。运行期必须保留 `bin/postgres`、`bin/initdb`、`bin/pg_ctl`、`bin/psql`（控制面依赖它执行 SQL）、`lib/` 共享库、`share/`（含 ICU 与时区数据）、pgvector 扩展文件。

   **尺寸预期（据实说明）**：PostgreSQL 无模块化构建，编译开关裁不掉核心特性，只能删可选组件。因此「几十 MB」只在**压缩后的安装包增量**上成立（约 60–90 MB）；落到磁盘是 150–250 MB 量级。若预期为「装完占盘几十兆」，该目标不可达，需调整预期。

2. **【已定·本轮不做】签名与公证**：不签名、不公证**可以安装运行**，只是 macOS 会拦（提示「无法验证开发者」，需右键打开或在系统设置放行，也可 `xattr -dr com.apple.quarantine` 清除隔离属性），Windows 会有 SmartScreen「未知发布者」警告。要做到首次打开无提示，才必须 Apple Developer 账号（签名 + 公证）与 Windows 代码签名证书。本轮不涉及，单列一轮。

3. **【已定】终端默认 shell**：按平台原生取值——Windows `pwsh` → `powershell`（另提供 `cmd` 选项），unix `$SHELL` → `/bin/sh`。移除自定义的 `XUANYU_SHELL`，同时消除渲染层把 unix 写死 `bash` 的不一致与 `TerminalShellKind` 的标签失真。
4. **【已定】Windows 的 OS 沙箱**：Windows 没有 `sandbox-exec` / `bwrap` 的等价原语，不做该项实现。需要改的是现状的 fail-closed 逻辑——开启 `sandbox.osSandbox.enabled` 后 Windows 会拒绝一切 exec，这是缺陷；Windows 侧该开关表现为「不适用」，unix 侧保持真沙箱。
5. **【已定】发布产物：每平台三种形态**（安装版 + 免安装 + 包管理器）：

   | 平台 | 安装版 | 免安装形态 | 包管理器 |
   | --- | --- | --- | --- |
   | Windows | `nsis` | `zip` | 需要企业批量部署时再加 `msi` |
   | macOS | `dmg` | `zip`（即 `.app` 压缩包） | — |
   | Linux | `deb` | `AppImage`、`tar.gz` | — |

   架构覆盖：macOS `x86_64-apple-darwin` + `aarch64-apple-darwin`；Windows / Linux `x64`。

6. **【已定】产物命名**：`XuanYu-Agent-<version>-<os>[-<arch>].<ext>`，例如 `XuanYu-Agent-0.1.0-preview-windows-setup.exe`、`XuanYu-Agent-0.1.0-preview-mac-arm64.dmg`、`XuanYu-Agent-0.1.0-preview-linux.AppImage`。mac 因双架构必须带 `arch`，其余平台可省。

7. **【已定】PostgreSQL 生命周期由应用持有**（取代原「精简版运行时行为」）：

   - 启动应用时若实例未运行则拉起，退出时停止；实例归当前用户所有，不注册系统服务。
   - 权限最小化：数据目录与 socket 目录仅当前用户可访问；只监听回环地址，不对外暴露。
   - 异常退出残留：进程被强杀时 `postmaster.pid` 会留下，启动前需识别并清理，不能直接失败。
   - 单实例：应用已有单实例锁，天然避免两个实例抢同一数据目录。
   - 端口固定；被占用时给出明确报错，不静默失败。
   - 配置由应用写死，首次启动无需用户介入，装完即可用。

8. **【已定】三端目录规范：完全按平台惯例**（决定 2.7 的改法）。配置、数据、缓存、日志四类目录各自遵循平台规范，`~/.xuanyu` 这一非标准锚点废弃。

   | 类别 | Windows | macOS | Linux |
   | --- | --- | --- | --- |
   | 配置 | `%APPDATA%\XuanYu\` | `~/Library/Application Support/XuanYu/` | `$XDG_CONFIG_HOME/xuanyu/`（默认 `~/.config/xuanyu/`） |
   | 数据 | `%LOCALAPPDATA%\XuanYu\` | `~/Library/Application Support/XuanYu/` | `$XDG_DATA_HOME/xuanyu/`（默认 `~/.local/share/xuanyu/`） |
   | 缓存 | `%LOCALAPPDATA%\XuanYu\Cache\` | `~/Library/Caches/XuanYu/` | `$XDG_CACHE_HOME/xuanyu/`（默认 `~/.cache/xuanyu/`） |
   | 日志 | `%LOCALAPPDATA%\XuanYu\logs\` | `~/Library/Logs/XuanYu/` | `$XDG_STATE_HOME/xuanyu/logs/`（默认 `~/.local/state/xuanyu/logs/`） |

   路径策略由 Rust 引擎持有（`directories` crate，或等价的手写实现），Electron 只消费引擎给出的值——PostgreSQL、sqlite、projects、plugins、日志的实际读写方全在引擎侧，`app.getPath()` 覆盖不到。

   **数据根的三级覆盖链**（缓存、日志、配置三类不参与覆盖）：① `XUANYU_DATA_DIR` 环境变量（显式覆盖：命令行与自动化）；② 配置 `appDataDir`（应用内设置）；③ 上表的平台默认。

   **安装目录只读**：三端都不往安装目录写数据。`launch.conf` 机制与 `index.ts:32-49` 的读取逻辑删除，数据落位回归配置文件这一条通道。

   **免安装形态的数据落位**：与安装版完全一致，同样走平台标准目录。「免安装」只表示没有安装步骤，不代表数据与程序同级。

   **mac 的便携能力**：`.app` 是不可写的签名包，不存在「程序旁边放 data 目录」的检测式便携。数据根统一由配置 `appDataDir` 决定，默认即上表的平台标准目录。

   **收敛点**：`paths.rs:21-28`（配置锚点）、`paths.rs:31-33`（数据默认）、新增通用缓存目录与日志目录函数（`paths.rs:76-91` 的 `postgres_cache_dir()` 随下载链路一并删除）。其余路径全部由它们派生。

   **后果**：`~/.xuanyu` 下的既有数据不再被识别；按第七节约定，不写任何迁移代码。

9. **【已定】三端安装形态**：

   - **Windows**：NSIS 向导（许可 → 安装模式 → 安装位置 → 进度 → 完成），可改安装位置。
   - **macOS**：`dmg` 挂载后展示许可、拖入 `Applications`，**没有安装位置选择**；不使用 `.pkg`（企业 MDM 场景，个人分发不需要）。
   - **Linux**：`AppImage` 无需安装（`chmod +x` 直接运行）；`deb` 由包管理器装到 `/opt`。

   数据库不再需要任何配置步骤，因此安装向导里不含数据库页，也不依赖应用内首次启动向导。

10. **【已定】随本次改造一并清理**（用户可见的数据库配置入口与随之失效的机制）：

    | 位置 | 清理内容 |
    | --- | --- |
    | `crates/xuanyu-cli/src/main.rs:72,84,210-299` | `portable-install` 与 `db-external` 两个子命令及其实现 |
    | `apps/desktop/build/installer.nsh:106-360,563-577` | 数据库配置页全部控件与函数、`customInstall` 里的便携安装调用；`installer.nsh:528-538` 写 `launch.conf` 的 `.onInstSuccess` |
    | `crates/xuanyu-storage/src/embedded_postgres.rs:15-39,214-265,712-765` | 下载源（URL / SHA256 / 镜像）、`Expand-Archive` 解压、按 `initdb.exe` 校验安装的链路 |
    | `crates/xuanyu-storage/src/paths.rs:71-91` | `postgres_cache_dir()` 与 `XUANYU_POSTGRES_CACHE_DIR`（不再下载，缓存无意义） |
    | `apps/desktop/src/main/index.ts:22-49` | `launch.conf` 读取逻辑 |
    | `crates/xuanyu-core/src/config_schema.rs:36-43,108-114`、`lib.rs:4174-4219` | `memory.postgres.connectionString` 的用户写入与校验路径（保留读取，供内部固定连接串） |
    | `crates/xuanyu-storage/src/paths.rs:41-55` | `resolve_app_dir_from_selection()`，无调用方 |
    | `README.md:106` + `i18n/README.{en,ja,es,fr}.md:106` | `db-external` 的描述 |
    | `crates/xuanyu-memory/src/lib.rs`、`crates/xuanyu-core/src/config_schema.rs:36-43` | `memory.backend` 多选收掉，固定 `postgres`；`sqlite` / `file` 后端取值与校验删除 |
    | `crates/xuanyu-storage/src/{paths.rs,lib.rs,control_plane.rs,control_plane_handle.rs,event_journal.rs}`、`Cargo.toml`、`xuanyu-core/src/{lib.rs,config_schema.rs}`、`xuanyu-session/src/lib.rs` | sqlite 控制面整体删除，控制面统一走 `postgres_control_plane.rs` |
    | `apps/desktop-ui/src/init/*`、设置面板 | 连接串/主机/端口/账号密码输入控件 |

    **内存后端固定为 PostgreSQL**：`memory.backend` 不再接受 `sqlite` / `file` / `tantivy` 作为后端取值。注意 `crates/xuanyu-memory/src/tantivy_store.rs` 是全文检索的**镜像索引**（`configure_tantivy` / `mirror_memory_to_tantivy`），与后端选择不是同一个轴，**保留**。

    **sqlite 控制面整体删除**：涉及的 11 个文件为 `crates/xuanyu-storage/src/{paths.rs,lib.rs,control_plane.rs,control_plane_handle.rs,event_journal.rs}`、`crates/xuanyu-storage/Cargo.toml`、`crates/xuanyu-core/src/{lib.rs,config_schema.rs}`、`crates/xuanyu-session/src/lib.rs`，以及两处测试。具体符号包括 `default_sqlite_database_path`、`init_sqlite_connection_string`、控制面与事件日志里的 sqlite 分支；控制面统一走 `postgres_control_plane.rs`。

---

## 六、建议实施顺序

**阶段 1 · 让三端能构建、能启动**
三端目录规范落地（2.7、决策 8：配置锚点 / 数据默认 / 缓存 / 日志 + 数据根三级链，清除 `launch.conf` 机制与孤立字段）→ `node-pty`（1.2）→ 脚本跨平台化（1.4，含 `smoke.ps1` 的 `--bin` 修正）→ 打包配置补齐（1.3、决策 5、决策 6）→ CI 加 mac/linux target 矩阵（1.5）→ 补 `.gitattributes`（三.1）。

**阶段 2 · 让三端功能完整**
内置 PostgreSQL 接入与三端实测（1.1、三.5，依赖决策 1：Windows 沿用 EDB、mac/linux 接 conda-forge，新增构建期获取脚本「下载 → 校验 → 剥离」，含 pgvector 与体积剥离）→ PostgreSQL 生命周期与权限最小化（决策 7）→ 清理用户数据库配置入口与失效机制（决策 10）→ 进程管理去 Windows 化（2.2）→ 沙箱三端补齐（2.1，依赖决策 4）→ 风险命令清单按平台分层（2.3）→ 终端 shell 策略统一（2.4，依赖决策 3）。

**阶段 3 · 发布质量**
图标三端齐备（含 `.icns`）→ mac/linux 许可展示 → README 与 i18n 口径更新（三.6、三.7）→ 三端回归。签名与公证不入本阶段，单独排一轮（决策 2）。

---

## 七、改造约定

- 平台差异按各平台原生写法直接分支，保持三端对等，不引入兼容层、降级路径或开关。
- 不写解释性、致歉性注释（例如「为支持 mac/linux 而…」），这些实现就是设计本身。
- 不修改既有的业务阈值与安全策略取值（如 2.3 的清单条目，只做平台分层，不调整严格程度）。
- 源码中不得存在孤立、无用的东西：无调用方的函数、只被界面展示而无人读取的配置字段、自造但没有实际作用的开关，一律删除（2.7 已列出的两处即属此类）。不留死代码，不做「备而不用」的预留。
- 不得为了测试而在源码中写入任何无用之物；测试与源码严格分离，依赖私有项的测试直接删除，不为测试暴露内部。