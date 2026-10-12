import type { EngineStatus } from '@shared/protocol';
import { EngineRpcClient } from './EngineRpcClient';

export interface EngineClient {
  getStatus(): EngineStatus;
  start(): Promise<EngineStatus>;
  stop(): Promise<EngineStatus>;
  restart(): Promise<EngineStatus>;
  getRpc(): EngineRpcClient;
  onStatusChange(listener: (status: EngineStatus) => void): () => void;
}

export type RustEngineClientOptions = {
  /** 便携式安装时由安装器写入：数据目录（数据库/插件/skill 等）位于安装目录\data。 */
  dataDir?: string;
};

export class RustEngineClient implements EngineClient {
  private status: EngineStatus = { state: 'stopped' };
  private readonly rpc: EngineRpcClient;
  private readonly statusListeners = new Set<(status: EngineStatus) => void>();
  private exitUnsub: (() => void) | null = null;
  private startPromise: Promise<EngineStatus> | null = null;
  /** 记录显式 stop/restart，期间子进程退出不应触发自动重启。 */
  private stopping = false;
  private crashCount = 0;
  private crashWindowStart = 0;
  private restartTimer: ReturnType<typeof setTimeout> | null = null;

  private static readonly CRASH_WINDOW_MS = 10_000;
  private static readonly CRASH_LIMIT = 3;
  private static readonly RESTART_DELAY_MS = 1_000;

  constructor(private readonly options: RustEngineClientOptions = {}) {
    this.rpc = new EngineRpcClient(this.options.dataDir ? { dataDir: this.options.dataDir } : {});
    this.exitUnsub = this.rpc.onProcessExit((info) => {
      this.setStatus({
        state: 'error',
        detail: `Engine exited (code=${info.code ?? 'null'}, signal=${info.signal ?? 'null'})`,
      });
      // 非预期退出（非显式 stop）时自动拉起引擎，带防循环窗口。
      if (!this.stopping) this.scheduleAutoRestart();
    });
  }

  getStatus(): EngineStatus {
    return this.status;
  }

  getRpc(): EngineRpcClient {
    return this.rpc;
  }

  onStatusChange(listener: (status: EngineStatus) => void): () => void {
    this.statusListeners.add(listener);
    return () => this.statusListeners.delete(listener);
  }

  async start(): Promise<EngineStatus> {
    this.stopping = false;
    this.cancelAutoRestart();
    if (this.status.state === 'ready') return this.status;
    if (this.startPromise) return this.startPromise;

    this.startPromise = this.doStart().finally(() => {
      this.startPromise = null;
    });
    return this.startPromise;
  }

  private async doStart(): Promise<EngineStatus> {
    this.setStatus({ state: 'starting' });
    try {
      const result = (await this.rpc.start()) as { protocolVersion?: string };
      this.setStatus({
        state: 'ready',
        version: result.protocolVersion ?? '0.1.0',
        detail: 'xuanyu-engine-server (stdio JSON-RPC)',
      });
      return this.status;
    } catch (err) {
      this.setStatus({
        state: 'error',
        detail: err instanceof Error ? err.message : String(err),
      });
      throw err;
    }
  }

  async stop(): Promise<EngineStatus> {
    this.stopping = true;
    this.cancelAutoRestart();
    this.crashCount = 0;
    this.crashWindowStart = 0;
    await this.rpc.stop();
    this.setStatus({ state: 'stopped' });
    return this.status;
  }

  async restart(): Promise<EngineStatus> {
    this.stopping = true;
    this.cancelAutoRestart();
    this.crashCount = 0;
    this.crashWindowStart = 0;
    try {
      await this.rpc.stop();
    } catch {
      // process may already be dead
    }
    this.setStatus({ state: 'stopped', detail: 'Restarting engine…' });
    return this.start();
  }

  /** 非预期退出后延迟自动重启；滑动窗口内连续崩溃超过 CRASH_LIMIT 次则放弃并告警。 */
  private scheduleAutoRestart(): void {
    const now = Date.now();
    if (now - this.crashWindowStart > RustEngineClient.CRASH_WINDOW_MS) {
      this.crashWindowStart = now;
      this.crashCount = 0;
    }
    this.crashCount += 1;
    if (this.crashCount > RustEngineClient.CRASH_LIMIT) {
      console.error(
        `[engine] engine crashed ${this.crashCount} times within ${RustEngineClient.CRASH_WINDOW_MS}ms, auto-restart disabled`,
      );
      return;
    }
    console.warn(
      `[engine] engine exited unexpectedly, auto-restarting (${this.crashCount}/${RustEngineClient.CRASH_LIMIT})`,
    );
    this.restartTimer = setTimeout(() => {
      this.restartTimer = null;
      void this.start().catch((err) => {
        console.error('[engine] auto-restart failed', err);
      });
    }, RustEngineClient.RESTART_DELAY_MS);
  }

  private cancelAutoRestart(): void {
    if (this.restartTimer !== null) {
      clearTimeout(this.restartTimer);
      this.restartTimer = null;
    }
  }

  private setStatus(next: EngineStatus): void {
    this.status = next;
    for (const listener of this.statusListeners) listener(next);
  }
}

/** @deprecated 仅测试占位，生产使用 RustEngineClient */
export class LocalEngineClient implements EngineClient {
  private status: EngineStatus = { state: 'stopped' };
  private readonly statusListeners = new Set<(status: EngineStatus) => void>();

  getStatus(): EngineStatus {
    return this.status;
  }

  getRpc(): EngineRpcClient {
    throw new Error('LocalEngineClient has no RPC');
  }

  onStatusChange(listener: (status: EngineStatus) => void): () => void {
    this.statusListeners.add(listener);
    return () => this.statusListeners.delete(listener);
  }

  async start(): Promise<EngineStatus> {
    this.status = { state: 'ready', version: 'local' };
    for (const listener of this.statusListeners) listener(this.status);
    return this.status;
  }

  async stop(): Promise<EngineStatus> {
    this.status = { state: 'stopped' };
    for (const listener of this.statusListeners) listener(this.status);
    return this.status;
  }

  async restart(): Promise<EngineStatus> {
    await this.stop();
    return this.start();
  }
}
