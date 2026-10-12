/**
 * 主区右上角悬浮：打开 / 收起底部终端。
 */

import React from 'react';
import { PanelBottom } from 'lucide-react';
import { useI18n } from '../../i18n';
import { useTerminalStore } from '../../stores/terminalStore';

export default function TerminalToggleFab() {
  const { t } = useI18n();
  const open = useTerminalStore((s) => s.open);
  const toggleOpen = useTerminalStore((s) => s.toggleOpen);

  return (
    <button
      type="button"
      className={`xy-terminal-fab${open ? ' is-active' : ''}`}
      onClick={toggleOpen}
      aria-label={t('terminal.toggle', '切换终端')}
      title={t('terminal.toggle', '切换终端')}
      aria-pressed={open}
    >
      <PanelBottom size={16} strokeWidth={1.75} />
    </button>
  );
}
