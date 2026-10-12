/**
 * BrandMark - 统一品牌标识（应用图标）。
 * 直接展示 assets/xuanyu.png（源 PNG 已含圆角容器符号），各门面位置共用。
 */

import React from 'react';

interface BrandMarkProps {
  size?: number;
  withWordmark?: boolean;
  /** 字号期望（仅 withWordmark 时生效），默认跟随 size */
  className?: string;
  ariaLabel?: string;
}

/** 只含图标的中性标识（无文字）。 */
export function BrandGlyph({
  size = 28,
  className,
  ariaLabel = 'XuanYu',
}: {
  size?: number;
  className?: string;
  ariaLabel?: string;
}): React.ReactElement {
  return (
    <img
      src="/assets/xuanyu.png"
      width={size}
      height={size}
      alt={ariaLabel}
      className={className}
      draggable={false}
      style={{ display: 'inline-block', objectFit: 'contain' }}
    />
  );
}

/** 图标 + 文字标锁子，供品牌区使用。 */
export default function BrandMark({
  size = 28,
  withWordmark = true,
  className,
  ariaLabel = 'XuanYu',
}: BrandMarkProps): React.ReactElement {
  return (
    <span className={className} style={{ display: 'inline-flex', alignItems: 'center', gap: 9 }}>
      <BrandGlyph size={size} ariaLabel={ariaLabel} />
      {withWordmark ? (
        <span
          style={{
            fontSize: size * 0.52,
            fontWeight: 600,
            letterSpacing: '0.04em',
            lineHeight: 1,
            color: 'currentColor',
            whiteSpace: 'nowrap',
          }}
        >
          XuanYu
        </span>
      ) : null}
    </span>
  );
}