/**
 * Cursor-like project picker for the composer chrome.
 * Empty: "选择项目" in full-width chrome. Open: search + recent list + new project submenu.
 */

import React, { useEffect, useMemo, useState } from 'react';

import { Input, Popover } from 'antd';
import {
  CloseOutlined,
  FolderOpenOutlined,
  FolderOutlined,
  PlusOutlined,
  RightOutlined,
  SearchOutlined,
} from '@ant-design/icons';

import { useI18n } from '../../i18n';
import {
  fsPathKey,
  normalizeRecentProjectPath,
} from '../../foundation/config/paths';

/** 被谁调用：项目选择 UI。做什么：从绝对路径取末段名。 */
export function basenamePath(value?: string): string {
  if (!value?.trim()) return '';
  const normalized = normalizeRecentProjectPath(value);
  const parts = normalized.split('/');
  return parts[parts.length - 1] || normalized;
}

interface ComposerProjectPickerProps {
  projectDir: string;
  projectLabel: string;
  recentProjects: string[];
  onSelect: (dir: string) => void;
  onRemoveRecent: (dir: string) => void;
  onClearProject: () => void;
  /** 已绑定项目的正式会话不展示叉号，避免半途解绑导致侧栏移位。 */
  allowClear?: boolean;
  onNewBlank: () => void;
  onOpenFolder: () => void;
  onMenuOpenChange?: (open: boolean) => void;
}

/** 被谁调用：ChatView composer。做什么：切换/记忆最近工作区，触发 workspace.open。 */
export default function ComposerProjectPicker({
  projectDir,
  projectLabel,
  recentProjects,
  onSelect,
  onRemoveRecent,
  onClearProject,
  allowClear = true,
  onNewBlank,
  onOpenFolder,
  onMenuOpenChange,
}: ComposerProjectPickerProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [query, setQuery] = useState('');

  useEffect(() => {
    if (!open) {
      setQuery('');
      setNewOpen(false);
    }
  }, [open]);

  useEffect(() => {
    onMenuOpenChange?.(open || newOpen);
  }, [open, newOpen, onMenuOpenChange]);

  const list = useMemo(() => {
    const seen = new Set<string>();
    const dirs: string[] = [];
    for (const dir of [
      ...(projectDir ? [projectDir] : []),
      ...recentProjects,
    ]) {
      const norm = normalizeRecentProjectPath(dir);
      const key = fsPathKey(norm);
      if (!norm || !key || seen.has(key)) continue;
      seen.add(key);
      dirs.push(norm);
    }
    const q = query.trim().toLowerCase();
    if (!q) return dirs;
    return dirs.filter((dir) => {
      const name = basenamePath(dir).toLowerCase();
      return name.includes(q) || dir.toLowerCase().includes(q);
    });
  }, [projectDir, recentProjects, query]);

  const hasProject = !!projectDir.trim();

  const newMenu = (
    <div className="xy-project-picker-panel xy-project-picker-sub">
      <button
        type="button"
        className="xy-project-picker-row"
        onClick={() => {
          setOpen(false);
          onNewBlank();
        }}
      >
        <PlusOutlined />
        <span>{t('chat.projectPicker.newBlank')}</span>
      </button>
      <button
        type="button"
        className="xy-project-picker-row"
        onClick={() => {
          setOpen(false);
          onOpenFolder();
        }}
      >
        <FolderOpenOutlined />
        <span>{t('chat.projectPicker.useFolder')}</span>
      </button>
    </div>
  );

  const panel = (
    <div className="xy-project-picker-panel">
      <div className="xy-project-picker-search">
        <SearchOutlined className="xy-project-picker-search-icon" />
        <Input
          allowClear
          bordered={false}
          size="small"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t('chat.projectPicker.search')}
          aria-label={t('chat.projectPicker.search')}
        />
      </div>

      <div className="xy-project-picker-list" role="listbox">
        {list.length === 0 ? (
          <div className="xy-project-picker-empty">{t('chat.projectEmpty')}</div>
        ) : (
          list.map((dir) => {
            const selected = fsPathKey(dir) === fsPathKey(projectDir);
            const name = basenamePath(dir);
            const shown = normalizeRecentProjectPath(dir);
            return (
              <div
                key={fsPathKey(shown) || shown || dir}
                role="option"
                aria-selected={selected}
                className={`xy-project-picker-row is-project${selected ? ' is-selected' : ''}`}
                onClick={() => {
                  setOpen(false);
                  onSelect(shown || dir);
                }}
              >
                <FolderOutlined className="xy-project-picker-row-icon" />
                <span className="xy-project-picker-row-label" title={shown}>
                  {name}
                </span>
                <button
                  type="button"
                  className="xy-project-picker-remove"
                  title={t('chat.projectPicker.remove')}
                  aria-label={t('chat.projectPicker.remove')}
                  onClick={(e) => {
                    e.stopPropagation();
                    onRemoveRecent(dir);
                  }}
                >
                  <CloseOutlined />
                </button>
              </div>
            );
          })
        )}
      </div>

      <div className="xy-project-picker-divider" />

      <Popover
        open={newOpen}
        onOpenChange={setNewOpen}
        trigger="hover"
        placement="rightTop"
        arrow={false}
        overlayClassName="xy-project-picker-overlay"
        content={newMenu}
      >
        <button type="button" className="xy-project-picker-row is-action">
          <PlusOutlined />
          <span>{t('chat.projectPicker.new')}</span>
          <RightOutlined className="xy-project-picker-chevron" />
        </button>
      </Popover>
    </div>
  );

  return (
    <div className={`xy-composer-project-slot${hasProject ? ' has-project' : ''}`}>
      <Popover
        open={open}
        onOpenChange={setOpen}
        trigger="click"
        placement="topLeft"
        arrow={false}
        overlayClassName="xy-project-picker-overlay"
        destroyTooltipOnHide
        content={panel}
      >
        <button
          type="button"
          className="xy-composer-chrome-item xy-composer-project-trigger"
          title={hasProject ? normalizeRecentProjectPath(projectDir) || projectLabel : t('chat.projectPicker.select')}
          aria-label={t('chat.projectPicker.select')}
        >
          <FolderOutlined />
          <span className="xy-composer-chrome-label">
            {hasProject ? projectLabel : t('chat.projectPicker.select')}
          </span>
        </button>
      </Popover>
      {hasProject && allowClear ? (
        <button
          type="button"
          className="xy-composer-project-close"
          title={t('chat.projectPicker.close')}
          aria-label={t('chat.projectPicker.close')}
          onClick={(e) => {
            e.stopPropagation();
            onClearProject();
          }}
        >
          <CloseOutlined />
        </button>
      ) : null}
    </div>
  );
}
