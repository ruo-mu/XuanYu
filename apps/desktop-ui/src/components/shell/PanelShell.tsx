/**
 * 管理页布局积木：页面容器、控制栏、数据分区、加载态。
 */

import React from 'react';
import { Spin } from 'antd';

/** 页面根容器 */
export function AdminPage({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={`xy-admin-page${className ? ` ${className}` : ''}`}>{children}</div>;
}

/** 页面主内容区；wide 时加宽最大宽度。 */
export function AdminContent({ children, wide }: { children: React.ReactNode; wide?: boolean }) {
  return <div className={`xy-admin-content${wide ? ' is-wide' : ''}`}>{children}</div>;
}

/** 单行控制区：搜索 / 筛选 / 操作 / 摘要数据 */
export function ControlBar({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={`xy-control-bar${className ? ` ${className}` : ''}`}>{children}</div>;
}

/** 控制栏左侧：搜索 / 筛选。 */
export function ControlBarStart({ children }: { children: React.ReactNode }) {
  return <div className="xy-control-bar-start">{children}</div>;
}

/** 控制栏右侧：操作按钮。 */
export function ControlBarEnd({ children }: { children: React.ReactNode }) {
  return <div className="xy-control-bar-end">{children}</div>;
}

/** 紧凑摘要，替代大指标卡 */
export function InlineStats({ children }: { children: React.ReactNode }) {
  return <div className="xy-inline-stats">{children}</div>;
}

/** 单条紧凑指标：数值 + 标签。 */
export function InlineStat({ label, value }: { label: React.ReactNode; value: React.ReactNode }) {
  return (
    <span className="xy-inline-stat">
      <span className="xy-inline-stat-value">{value}</span>
      <span className="xy-inline-stat-label">{label}</span>
    </span>
  );
}

/** 主内容区 — 单块毛玻璃表面，避免卡片套卡片 */
export function PageSurface({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={`xy-page-surface${className ? ` ${className}` : ''}`}>{children}</div>;
}

/** 分区：标题行 + 内容，不再套 Ant Card */
export function DataBlock({
  title,
  extra,
  children,
  flush,
}: {
  title?: React.ReactNode;
  extra?: React.ReactNode;
  children: React.ReactNode;
  flush?: boolean;
}) {
  return (
    <section className={`xy-data-block${flush ? ' is-flush' : ''}`}>
      {(title || extra) && (
        <div className="xy-data-block-head">
          {title ? <h3 className="xy-data-block-title">{title}</h3> : <span />}
          {extra ? <div className="xy-data-block-extra">{extra}</div> : null}
        </div>
      )}
      <div className="xy-data-block-body">{children}</div>
    </section>
  );
}

/** 懒加载页面的全屏加载占位。 */
export function WorkspaceLoading({ label }: { label: string }) {
  return (
    <div className="xy-workspace-loading">
      <div className="xy-workspace-loading-ring" aria-hidden />
      <Spin size="large" />
      <span>{label}</span>
    </div>
  );
}
