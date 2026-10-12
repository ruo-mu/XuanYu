/**
 * 初始化向导草稿的 sessionStorage 读写（刷新页面不丢当前步骤）。
 */

import type { InitDraft } from './initApi';

const STORAGE_KEY = 'xy-init-wizard-state';

/** sessionStorage 里保存的向导草稿与步骤序号。 */
export interface InitWizardPersistedState {
  draft: InitDraft;
  step: number;
}

/** 读取向导草稿；无效或缺失时返回 null。 */
export function loadInitWizardState(): InitWizardPersistedState | null {
  try {
    const raw = sessionStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as InitWizardPersistedState;
    if (!parsed?.draft?.config || typeof parsed.step !== 'number') return null;
    return { draft: parsed.draft, step: parsed.step };
  } catch {
    return null;
  }
}

/** 写入当前向导草稿与步骤。 */
export function saveInitWizardState(state: InitWizardPersistedState): void {
  try {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* ignore quota / private mode */
  }
}

/** 清除向导草稿（完成后或放弃时）。 */
export function clearInitWizardState(): void {
  try {
    sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    /* ignore */
  }
}
