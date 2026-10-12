/**
 * 工作台二级导航：胶囊分段控件，用于同页切 section。
 */

import React from 'react';

/** 二级导航一项。 */
export interface SubNavItem {
  key: string;
  label: string;
}

interface SubNavProps {
  items: SubNavItem[];
  value: string;
  onChange: (key: string) => void;
  className?: string;
}

/** 工作台二级导航 — 胶囊分段控件 */
export default function SubNav({ items, value, onChange, className }: SubNavProps) {
  return (
    <nav className={`xy-workspace-tabs${className ? ` ${className}` : ''}`} aria-label="Section navigation">
      <div className="xy-workspace-tabs-track">
        {items.map((item) => (
          <button
            key={item.key}
            type="button"
            className={`xy-workspace-tab${value === item.key ? ' is-active' : ''}`}
            onClick={() => onChange(item.key)}
            aria-current={value === item.key ? 'page' : undefined}
          >
            {item.label}
          </button>
        ))}
      </div>
    </nav>
  );
}
