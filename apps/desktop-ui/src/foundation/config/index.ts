/**
 * 配置入口：类型、工具启用与工作区偏好。
 * （Desktop 无 Zod schema；运行时由 Rust 合并/读取。）
 */

export * from './types';
export * from './toolEnablement';
export * from './paths';
export {
  restoreWorkspaceFromConfig,
  buildWorkspacePreferenceUpdate,
  buildWorkspaceForgetUpdate,
} from './workspacePreference';
