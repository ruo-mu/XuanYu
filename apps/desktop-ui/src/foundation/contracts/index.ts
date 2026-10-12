/**
 * 基础契约：运行时身份、checkpoint，以及沙箱、共享类型与 token 用量。
 */

/** 一次运行的身份：runId 与启动时间。 */
export interface RuntimeIdentity {
  runId: string;
  startedAt: string;
}

/** 运行时检查点：归属 session/task/runtime，载荷任意。 */
export interface RuntimeCheckpoint<TState = unknown> {
  id: string;
  owner: 'session' | 'task' | 'runtime';
  state: TState;
  createdAt: string;
}

export * from './sandbox';
export * from './sharedTypes';
export * from './tokenUsage';
export * from './wireSession';
export * from './contextStatus';
export * from './contextLoadPolicy';
export * from './modelPricing';
export * from './toolDefinition';
