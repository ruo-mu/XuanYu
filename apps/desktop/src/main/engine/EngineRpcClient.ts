import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import { join } from 'node:path';
import { app } from 'electron';
import { existsSync } from 'node:fs';

type RpcRequest = {
  id: string;
  method: string;
  params?: unknown;
};

type RpcResponse = {
  id: string;
  ok: boolean;
  result?: unknown;
  error?: string;
};

export type EngineRpcEvent = {
  channel: 'event';
  event: string;
  payload: unknown;
};

type Pending = {
  resolve: (value: unknown) => void;
  reject: (error: Error) => void;
};

export type EngineRpcClientOptions = {
  binaryPath?: string;
  dataDir?: string;
};

export class EngineRpcClient {
  private child: ChildProcessWithoutNullStreams | null = null;
  private nextId = 1;
  private readonly pending = new Map<string, Pending>();
  private readonly listeners = new Set<(event: EngineRpcEvent) => void>();
  private started = false;

  constructor(private readonly options: EngineRpcClientOptions = {}) {}

  onEvent(listener: (event: EngineRpcEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async start(): Promise<unknown> {
    if (this.started) return this.request('engine.status', {});

    const binary = this.options.binaryPath ?? resolveEngineBinary();
    if (!existsSync(binary)) {
      throw new Error(`Engine binary not found: ${binary}`);
    }

    this.child = spawn(binary, [], {
      stdio: ['pipe', 'pipe', 'pipe'],
      env: {
        ...process.env,
        ...(this.options.dataDir ? { XUANYU_DATA_DIR: this.options.dataDir } : {}),
      },
      windowsHide: true,
    });

    const stdout = createInterface({ input: this.child.stdout });
    stdout.on('line', (line) => {
      try {
        const response = JSON.parse(line) as RpcResponse;
        const pending = this.pending.get(response.id);
        if (!pending) return;
        this.pending.delete(response.id);
        if (response.ok) pending.resolve(response.result);
        else pending.reject(new Error(response.error || 'engine rpc failed'));
      } catch (err) {
        console.error('[engine-rpc] invalid stdout line', line, err);
      }
    });

    const stderr = createInterface({ input: this.child.stderr });
    stderr.on('line', (line) => {
      try {
        const event = JSON.parse(line) as EngineRpcEvent;
        if (event.channel !== 'event') return;
        for (const listener of this.listeners) listener(event);
      } catch {
        // tracing logs on stderr are not events
      }
    });

    this.child.on('exit', (code, signal) => {
      this.started = false;
      this.child = null;
      const reason = `Engine exited (code=${code ?? 'null'}, signal=${signal ?? 'null'})`;
      for (const [, pending] of this.pending) pending.reject(new Error(reason));
      this.pending.clear();
      for (const listener of this.exitListeners) {
        listener({ code, signal });
      }
    });

    this.started = true;
    return this.request('engine.init', {});
  }

  private readonly exitListeners = new Set<
    (info: { code: number | null; signal: NodeJS.Signals | null }) => void
  >();

  onProcessExit(
    listener: (info: { code: number | null; signal: NodeJS.Signals | null }) => void,
  ): () => void {
    this.exitListeners.add(listener);
    return () => this.exitListeners.delete(listener);
  }

  async stop(): Promise<void> {
    if (!this.child) return;
    try {
      await this.request('engine.shutdown', {});
    } catch {
      // process may exit before response
    }
    this.child?.kill();
    this.child = null;
    this.started = false;
  }

  async request<T = unknown>(method: string, params?: unknown): Promise<T> {
    if (!this.child?.stdin.writable) {
      throw new Error('Engine process is not running');
    }
    const id = String(this.nextId++);
    const payload: RpcRequest = { id, method, params };
    const line = `${JSON.stringify(payload)}\n`;

    const result = new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (value) => resolve(value as T),
        reject,
      });
    });

    this.child.stdin.write(line);
    return result;
  }
}

export function resolveEngineBinary(): string {
  const override = process.env.XUANYU_ENGINE_SERVER_BIN;
  if (override) return override;

  const name = process.platform === 'win32' ? 'xuanyu-engine-server.exe' : 'xuanyu-engine-server';

  if (app.isPackaged) {
    return join(process.resourcesPath, 'bin', name);
  }

  const candidates = [
    join(app.getAppPath(), 'resources', 'bin', name),
    join(app.getAppPath(), '../../../target/debug', name),
    join(app.getAppPath(), '../../../target/release', name),
    join(process.cwd(), 'target/debug', name),
    join(process.cwd(), 'target/release', name),
    // dev 模式下 cwd 可能是 apps/desktop，向上两级到仓库根
    join(process.cwd(), '..', '..', 'target/debug', name),
    join(process.cwd(), '..', '..', 'target/release', name),
  ];

  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate;
  }

  return join(process.cwd(), 'target/debug', name);
}
