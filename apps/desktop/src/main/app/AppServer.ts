import type { EngineClient } from '../engine/EngineClient';

export class AppServer {
  constructor(readonly engine: EngineClient) {}

  async coreInvoke(command: string, args: unknown = {}): Promise<unknown> {
    const state = this.engine.getStatus().state;
    if (state === 'error' || state === 'stopped') {
      await this.engine.restart();
    } else if (state !== 'ready') {
      await this.engine.start();
    }
    return this.engine.getRpc().request('core.invoke', { command, args });
  }
}
