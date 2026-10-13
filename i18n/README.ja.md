<p align="center">
  <img src="../apps/desktop-ui/public/assets/xuanyu.png" alt="XuanYu Agent" width="180" />
</p>

<h1 align="center">XuanYu Agent · 玄语</h1>

<p align="center">
  <strong>ローカルファーストのデスクトップ AI エージェントワークスペース</strong><br/>
  <sub>推論とオーケストレーションは独立した Rust エンジン上で動作し、記憶とデータはあなた自身のマシンに留まります</sub>
</p>

<p align="center">
  <a href="../README.md"><img alt="简体中文" src="https://img.shields.io/badge/简体中文-d9d9d9"></a>
  <a href="./README.en.md"><img alt="English" src="https://img.shields.io/badge/English-d9d9d9"></a>
  <a href="./README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/日本語-d9d9d9"></a>
  <a href="./README.es.md"><img alt="Español" src="https://img.shields.io/badge/Español-d9d9d9"></a>
  <a href="./README.fr.md"><img alt="Français" src="https://img.shields.io/badge/Français-d9d9d9"></a>
</p>

<p align="center">
  <a href="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue"><img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue" alt="Platform"/></a>
  <a href="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9"><img src="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9" alt="Version"/></a>
  <a href="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6"><img src="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6" alt="Tech Stack"/></a>
  <a href="https://img.shields.io/badge/engine-15%20crates-16a34a"><img src="https://img.shields.io/badge/engine-15%20crates-16a34a" alt="Cargo Workspace"/></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="License"/></a>
</p>

---

**玄語（XuanYu Agent）** は **ローカルファースト**のデスクトップ AI エージェントワークスペースで、Windows・macOS・Linux にネイティブ対応します。中核となる推論・ツール実行・安全ガバナンスは独立したネイティブ Rust エンジンプロセス上で動作し、Electron は UI とデバイス連携のみを担います。セッション・長期記憶・実行ログはすべて**成果物に同梱された PostgreSQL**（pgvector 込み）に保存されます——オフラインで利用でき、データは本機から出ません。同時に、任意の主要モデルやサードパーティの機能を接続できます。

> [!NOTE]
> 現在は `0.1.0-preview` プレビュー版で、**Windows 10+ / macOS（x64・arm64）/ Linux（x64）** にネイティブ対応しています。

## 目次

- [主な機能](#主な機能)
- [アーキテクチャ概要](#アーキテクチャ概要)
- [ワークスペース構成](#ワークスペース構成)
- [コアの仕組み](#コアの仕組み)
- [クイックスタート](#クイックスタート)
- [リリースパッケージのビルド](#リリースパッケージのビルド)
- [動作環境](#動作環境)
- [設定](#設定)
- [セキュリティモデル](#セキュリティモデル)
- [ロードマップ](#ロードマップ)
- [コミュニティとコントリビューション](#コミュニティとコントリビューション)
- [ライセンス](#ライセンス)

## 主な機能

- **ローカルネイティブエンジン** — 推論とオーケストレーションは独立プロセス `xuanyu-engine-server` 上で動作し、UI から完全に分離されています。エンジンのクラッシュはシェル側で検知・自動再起動でき、UI を伴わず単独で再利用することも可能です（CLI / ヘッドレスサービス）。
- **長期記憶** — PostgreSQL + pgvector による意味検索を主軸に、全文検索とローカル Tantivy インデックスを組み合わせたマルチパス・リコールで、セッションをまたいだ記憶と重要事実の蓄積を実現します。
- **プラグイン / MCP / スキルマーケット** — アプリ内でプラグイン、MCP サービス、スキルを閲覧・インストールし、コードを変更せずに Agent の能力を拡張できます。
- **マルチモデル接続** — セットアップウィザードが主要 Provider への接続をガイドし、タスク（タイトル / 計画 / 実装 / レビューなど）ごとに異なるモデルへルーティングできるため、コストと効果を両立できます。
- **マルチ Agent 協調** — サブ Agent によるタスク分解と、「レビュー - マージ」ワークフローで複雑なエンジニアリングタスクを完遂します。
- **マルチチャネル接続** — 微信、飛書、釘釘、Telegram、Discord、QQ、Line、WhatsApp などの外部 IM を、同一のセッションと記憶基盤に統合します。
- **安全ガバナンス** — コマンドブラックリスト、git 破壊的コマンドのレベル分け、パススコープ検証、OS サンドボックスを備え、高リスク操作は必ず人手による承認を要求します。
- **巻き戻し可能な編集再送信** — 履歴中の任意のユーザーメッセージを編集して再送信すると、AI がそのターンで生成したファイル変更のみを正確に取り消します（ファイル単位の内容スナップショット + 差分ロールバック）。未コミットの無関係な作業は保持されます。
- **パフォーマンスへの配慮** — 投機的実行によるツール往復レイテンシの隠蔽、コンテキストの自動圧縮、切り詰め時の継続により、長時間タスクが中断しません。
- **モダンなデスクトップ体験** — Electron + antd、ライト / ダークテーマ、ストリーミング Markdown、コードハイライト、Diff ビューを内蔵。

## アーキテクチャ概要

```text
レンダリング層 (React / Vite / antd)
        │  IPC / JSON-RPC
        ▼
Rust エンジン (xuanyu-engine-server)
        │  SQL
        ▼
PostgreSQL + pgvector
```

3 つの設計方針：

- **プロセス分離** — エンジンは独立プロセスで、stdin/stdout 上の JSON-RPC を介してデスクトップシェルと双方向通信します（[engine_server.rs](../crates/xuanyu-cli/src/bin/engine_server.rs) を参照）。UI に依存せず、単独で実行・テストできます。
- **イベント駆動** — エンジン内のあらゆる非同期変化（ターン差分、ツール進捗、権限リクエスト、記憶更新など）は `eventbus` を経由し、`AppEvent` にシリアライズされて UI へ主動的にプッシュされます。UI はポーリングせず、マルチクライアント購読に自然に対応します。
- **ストレージの単一情報源** — セッション、メッセージ、記憶、実行ログ、イベントログはすべて同梱の PostgreSQL に保存され、同一のスキーマを使用します。

## ワークスペース構成

```text
apps/
  desktop/            Electron シェル（メインプロセス / preload / パッケージング設定）
  desktop-ui/         レンダリング層（React + Vite + antd）
crates/               15 個の Rust crate（Cargo workspace）
proto/                エンジンとシェル間のプロトコル定義（app.proto / session.proto）
scripts/              IPC コード生成、パッケージングとスモークスクリプト
```

エンジン側の crate 一覧：

| Crate | 役割 |
|-------|------|
| `xuanyu-core` | エンジン統合層：AppCore、セッション / 記憶 / マーケット / チャネル |
| `xuanyu-execution` | 実行ランタイム：メインループ / ツールループ / コンテキスト組み立て / トークン予算 |
| `xuanyu-session` | セッション領域：SessionManager / スレッド / 分離 / メッセージキュー |
| `xuanyu-memory` | 長期記憶：pgvector 意味検索 + FTS + Tantivy + embedding |
| `xuanyu-tools` | ツールランタイム：MCP / リモート MCP / OS サンドボックス / 出力境界 |
| `xuanyu-governance` | 安全ガバナンス：コマンド検証 / git レベル分け / パス検証 / ロールバック / 記帳 |
| `xuanyu-channels` | 外部チャネル：微信 / 飛書 / 釘釘 / Telegram / Discord など |
| `xuanyu-eventbus` | イベントバス：spine / router / transformer / backpressure |
| `xuanyu-storage` | ストレージとコントロールプレーン：コネクションプール / セッションテーブル / 組み込み PostgreSQL |
| `xuanyu-protocol` | プロトコル契約：Envelope / AppEvent / AppError / DTO |
| `xuanyu-cli` | CLI と engine-server |
| `xuanyu-desktop-host` | デスクトップホストブリッジ：自動転写可能な IPC コマンドを公開 |
| `xuanyu-agent-bridge` | Agent ブリッジ：サブ Agent ワークスペースのバインディング |
| `xuanyu-agents` | サブ Agent ランタイム：watchdog / assess_merge |
| `xuanyu-extensions` | 能力拡張：プラグイン / MCP / スキルのロード |

## コアの仕組み

### 1 回のセッションのライフサイクル

ユーザーメッセージは [AppCore](../crates/xuanyu-core/src/lib.rs) に入った後、4 つのステップを経ます：

1. **コンテキスト組み立て** — [prepare_turn](../crates/xuanyu-execution/src/context_assembler.rs) が `loadMode`（lazy / balanced / eager）に基づき、そのターンで注入する内容（履歴、記憶検索結果、重要事実など）を決定します。
2. **メインループ** — [run_main_turn_loop](../crates/xuanyu-execution/src/main_turn_loop.rs) が反復上限内でモデル呼び出しとツール呼び出しの処理を繰り返します。ウォールクロックのデッドライン `deadline_ms` と反復のソフト上限をサポートします。
3. **ツールディスパッチ** — ネイティブのツール呼び出し、または「プリント式」ツール呼び出し（tool call をネイティブサポートしないモデル向けの互換機構）を検出し、`xuanyu-tools` に実行を委譲、結果をトランスクリプトに書き戻して次のラウンドへ進みます。
4. **確定** — モデルがツールを要求しなくなったら、`final_text` でそのターンを終了します。

重要なエンジニアリング上のポイント：

- **コンテキスト圧縮** — 予算の定数は [session_context.rs](../crates/xuanyu-execution/src/session_context.rs) を参照。設定 `contextManagement.autoCompactThreshold` によりトリガーされ、古いツール結果を刈り込んで handoff summary を追加し、情報の断絶を防ぎます。
- **切り詰めても継続** — `finish_reason` が `length` / `max_tokens` の場合は最終回答とみなさず、継続プロンプトを注入して次のラウンドへ進みます。
- **投機的実行** — [speculative_tools.rs](../crates/xuanyu-core/src/speculative_tools.rs) はストリーミング出力中にパラメータが確定した時点で実行キューに投入し、dispatch 時に直接結果を取得してツール往復レイテンシを隠蔽します。

### 長期記憶

[xuanyu-memory](../crates/xuanyu-memory/src) が担う、マルチバックエンド・マルチリコール経路のメモリスタック：

- `embedding` は 3 種類のソースをサポート：OpenAI 互換 API、ローカル推論、オフラインハッシュ（デフォルト。API Key がなくてもすぐに利用可能）。
- pgvector ベクトル検索を主軸（コサイン距離 Top-N）。連続失敗が 3 回以上で自動的にサーキットブレークし、PostgreSQL FTS のキーワード検索へフォールバックします。
- ローカル Tantivy インデックス（[tantivy_store.rs](../crates/xuanyu-memory/src/tantivy_store.rs)）を、完全オフライン時の最終リコール手段として使用します。

### フロントエンドとバックエンドの通信

プロトコル契約は [xuanyu-protocol](../crates/xuanyu-protocol/src/lib.rs) を参照：ジェネリックエンベロープ `Envelope<T>` が `correlation_id` / `trace_id` を保持してリクエスト相関とトレーシングを行い、統一された `AppError`（error code / recoverable / suggestion を含む）が生のエラーコードに取って代わります。

フロントエンドからエンジンへの IPC ブリッジはスクリプトで自動生成され、手書きによる契約のドリフトを防ぎます：

| スクリプト | 役割 |
|------|------|
| [gen-desktop-dispatch.py](../scripts/gen-desktop-dispatch.py) | `xuanyu-desktop-host` の関数シグネチャを解析し、dispatch ラッパーを生成 |
| [gen-desktop-ipc-wrappers.py](../scripts/gen-desktop-ipc-wrappers.py) | TypeScript 呼び出しラッパーを生成 |
| [gen-ipc-command-map.py](../scripts/gen-ipc-command-map.py) | 両側のコマンド網羅性を検証 |

エンジンの能力を変更した後は `pnpm gen:desktop-ipc` を実行するだけで、フロントエンドとバックエンドの契約を揃えられます。

## クイックスタート

### そのまま使う（推奨）

[Releases](https://github.com/ruo-mu/XuanYu/releases) からお使いのプラットフォーム向けの成果物をダウンロードすれば、インストール後すぐに使えます。初回起動時はセットアップウィザードが表示され、モデルを選択するだけで利用を開始できます。

| プラットフォーム | インストーラー | ポータブル |
|------|------|------|
| Windows | `nsis` | `zip` |
| macOS | `dmg` | `zip` |
| Linux | `deb` | `AppImage`、`tar.gz` |

成果物は `XuanYu-Agent-<バージョン>-<os>[-<arch>].<ext>` の命名規則に従います（例：`XuanYu-Agent-0.1.0-preview-windows-setup.exe`、`XuanYu-Agent-0.1.0-preview-mac-arm64.dmg`、`XuanYu-Agent-0.1.0-preview-linux.AppImage`）。macOS は 2 アーキテクチャのため `arch` を付け、その他のプラットフォームでは省略できます。ポータブル形態もインストーラー版と同じプラットフォーム標準ディレクトリを使い、データをインストール先には書き込みません。

PostgreSQL + pgvector は成果物に同梱されます。ビルド時に取得し、インストール時も実行時もダウンロード・展開は行いません。接続パラメータはアプリ側で固定されるため、接続文字列・ホスト・ポート・認証情報の設定は不要で、初回起動にネットワークも要しません。

インストーラーは今回コード署名・公証を行っていません。macOS では初回起動が Gatekeeper にブロックされ（右クリックから「開く」で許可）、Windows では SmartScreen の「不明な発行元」警告が表示されます。

### ソースから開発する

前提：Rust stable、Node.js 22+、pnpm。

```bash
pnpm install
pnpm dev          # Electron とローカルエンジンを起動
```

その他のよく使うコマンド：

```bash
pnpm typecheck    # メインプロセス + レンダリング層の型チェック
pnpm smoke        # スモークテスト
pnpm gen:desktop-ipc   # エンジン能力の変更後に IPC 契約を再生成
```

## リリースパッケージのビルド

```bash
pnpm dist
```

このコマンドは順に、release モードでエンジンをコンパイル → フロントエンドをビルド → 現在のプラットフォーム向けの配布成果物（Windows `nsis` / `zip`、macOS `dmg` / `zip`、Linux `AppImage` / `deb` / `tar.gz`）をパッケージング、を実行します。成果物は `apps/desktop/release-dist/` に出力されます。

> [!IMPORTANT]
> `pnpm install` を再実行すると、electron-builder の NSIS テンプレートパッチが上書きされます。リポジトリには `postinstall` フックが設定済みで自動的に再適用されますが、インストーラーに異常がある場合はまずパッチが有効かを確認してください。

## 動作環境

| カテゴリ | 要件 |
|------|------|
| オペレーティングシステム | Windows 10+ / macOS（x64・arm64）/ Linux（x64） |
| ランタイム | PostgreSQL 15+ と pgvector を成果物に同梱。利用者側でのインストールは不要 |
| 開発 | Node.js 22+、pnpm 10+、Rust stable |

## 設定

設定・データ・キャッシュ・ログは各プラットフォームの標準ディレクトリに従います：

| 種別 | Windows | macOS | Linux |
|------|---------|-------|-------|
| 設定 | `%APPDATA%\XuanYu\` | `~/Library/Application Support/XuanYu/` | `$XDG_CONFIG_HOME/xuanyu/`（既定 `~/.config/xuanyu/`） |
| データ | `%LOCALAPPDATA%\XuanYu\` | `~/Library/Application Support/XuanYu/` | `$XDG_DATA_HOME/xuanyu/`（既定 `~/.local/share/xuanyu/`） |
| キャッシュ | `%LOCALAPPDATA%\XuanYu\Cache\` | `~/Library/Caches/XuanYu/` | `$XDG_CACHE_HOME/xuanyu/`（既定 `~/.cache/xuanyu/`） |
| ログ | `%LOCALAPPDATA%\XuanYu\logs\` | `~/Library/Logs/XuanYu/` | `$XDG_STATE_HOME/xuanyu/logs/`（既定 `~/.local/state/xuanyu/logs/`） |

旧アンカー `~/.xuanyu` は廃止されました。

セットアップウィザードへは 2 つの入口からアクセスできます：タイトルバーの**「ファイル」メニュー**、またはシステム設定ページの**「セットアップウィザード」ボタン**です——初回設定後もいつでも再入場できます。データベース接続はアプリに組み込まれた固定パラメータを用いるため、接続文字列・ホスト・ポート・認証情報などの設定項目はすべて削除されています。設定の検証ロジックは [config_schema.rs](../crates/xuanyu-core/src/config_schema.rs) を参照。

## セキュリティモデル

- **コマンドブラックリスト** — `rm -rf /`、`sudo rm`、`chmod 777`、`eval` などの危険なコマンドを遮断します（[command_validator.rs](../crates/xuanyu-governance/src/command_validator.rs)）。
- **git レベル分け** — 破壊的コマンド（`rebase` / `reset` / `clean` / `filter-*` など）は一律で実行を拒否します（[git_command_policy.rs](../crates/xuanyu-governance/src/git_command_policy.rs)）。
- **パス検証** — ファイル操作をセッション分離スコープ内に制限します（[path_validator.rs](../crates/xuanyu-governance/src/path_validator.rs)）。
- **OS サンドボックス** — macOS は `sandbox-exec`、Linux は `bwrap` でコマンドをラップします。Windows には同等のプロセスラッパーがないため、このスイッチは Windows では適用されません（[os_sandbox.rs](../crates/xuanyu-tools/src/os_sandbox.rs)）。
- **人手による承認** — 高リスクなツール呼び出しは `PermissionRequested` をトリガーし、UI のダイアログで承認された後に `PermissionAnswered` で復帰、全行程を記帳します（[turn_ledger.rs](../crates/xuanyu-governance/src/turn_ledger.rs)）。
- **ロールバックの安全性** — ワークスペースの巻き戻しは内容スナップショットの差分復元を採用し、`git reset --hard` などの破壊的コマンドを**決して使用しません**。ユーザーが手動で変更したファイルは復元をスキップし、コンフリクト一覧に記載されます。

セキュリティ問題の報告方法は [SECURITY.md](../.github/SECURITY.md) を参照してください。

## ロードマップ

- [ ] リアルタイムセッションとイベント駆動サブスクリプション
- [ ] Agent / スキルマーケットのオンライン配信と自動更新
- [ ] より充実したレビューと権限ポリシーのテンプレート
- [ ] コールドスタートとパフォーマンスの最適化
- [ ] 記憶の可視化とハイブリッド検索のリランキング

## コミュニティとコントリビューション

- [CONTRIBUTING.md](../.github/CONTRIBUTING.md) — 開発に参加するための規約と手順
- [CODE_OF_CONDUCT.md](../.github/CODE_OF_CONDUCT.md) — コミュニティ行動規範
- [SECURITY.md](../.github/SECURITY.md) — セキュリティ問題の報告方法
- [Issue テンプレート](../.github/ISSUE_TEMPLATE) — バグ報告と機能提案

バグを見つけた場合や改善提案がある場合は、Issue または Pull Request をお送りください。

## ライセンス

本プロジェクトは [MIT License](../LICENSE) を採用しています。