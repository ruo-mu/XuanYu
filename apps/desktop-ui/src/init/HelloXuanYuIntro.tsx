/**
 * 开屏动画：霓虹描边的 "hello xuanyu" 字样。
 *
 * 主程序启动时铺满窗口播放，播完（或用户点击）回调 onDone 淡出。
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';

import {
  HELLO_XUANYU_STROKES,
  INTRO_DRAW,
  INTRO_DURATION_MS,
  INTRO_VIEWBOX,
  RAINBOW_GRADIENT,
  RAINBOW_GRADIENT_BOUNDS,
} from './helloXuanYuSvg';
import './helloXuanYuIntro.css';

/** 播放 INTRO_DURATION_MS 后自动结束；点击可提前跳过。 */
export default function HelloXuanYuIntro({ onDone }: { onDone: () => void }) {
  const doneRef = useRef(false);
  const pathRefs = useRef<Array<SVGPathElement | null>>([]);
  const [ready, setReady] = useState(false);

  const finish = useCallback(() => {
    if (doneRef.current) return;
    doneRef.current = true;
    onDone();
  }, [onDone]);

  useEffect(() => {
    if (!ready) return undefined;
    const doneTimer = window.setTimeout(finish, INTRO_DURATION_MS);
    return () => window.clearTimeout(doneTimer);
  }, [finish, ready]);

  useEffect(() => {
    pathRefs.current.forEach((node) => {
      if (!node) return;
      const length = node.getTotalLength();
      node.style.setProperty('--xy-stroke-len', `${length}`);
      node.style.strokeDasharray = `${length}`;
      node.style.strokeDashoffset = `${length}`;
    });
    requestAnimationFrame(() => setReady(true));
  }, []);

  const drawStyle = {
    '--xy-intro-easing': INTRO_DRAW.easing,
  } as React.CSSProperties;

  const strokeLayers = [
    { key: 'glow', className: 'xy-intro-stroke-glow', widthKey: 'glowWidth' as const },
    { key: 'mid', className: 'xy-intro-stroke-mid', widthKey: 'midWidth' as const },
    { key: 'core', className: 'xy-intro-stroke-core', widthKey: 'coreWidth' as const },
  ];

  let refIndex = 0;

  return (
    <div className="xy-init-intro" role="presentation" onClick={finish}>
      <div className="xy-init-intro-glow" aria-hidden />
      <div className={`xy-init-lottie${ready ? ' is-ready' : ''}`} role="img" aria-label="hello xuanyu">
        <svg viewBox={`0 0 ${INTRO_VIEWBOX.w} ${INTRO_VIEWBOX.h}`} aria-hidden="true">
          <defs>
            <filter id="xy-intro-neon" x="-80%" y="-80%" width="260%" height="260%">
              <feGaussianBlur stdDeviation="6" result="blur" />
              <feMerge>
                <feMergeNode in="blur" />
                <feMergeNode in="SourceGraphic" />
              </feMerge>
            </filter>
            <linearGradient
              id={RAINBOW_GRADIENT.id}
              gradientUnits="userSpaceOnUse"
              x1={RAINBOW_GRADIENT_BOUNDS.x1}
              y1={INTRO_VIEWBOX.h / 2}
              x2={RAINBOW_GRADIENT_BOUNDS.x2}
              y2={INTRO_VIEWBOX.h / 2}
            >
              {RAINBOW_GRADIENT.stops.map((stop) => (
                <stop key={stop.offset} offset={stop.offset} stopColor={stop.color} />
              ))}
            </linearGradient>
          </defs>
          <g className="xy-intro-stroke-stack" style={drawStyle}>
            {HELLO_XUANYU_STROKES.flatMap((stroke) =>
              strokeLayers.map((layer) => {
                const idx = refIndex;
                refIndex += 1;
                return (
                  <path
                    key={`${stroke.id}-${layer.key}`}
                    ref={(node) => { pathRefs.current[idx] = node; }}
                    className={`xy-intro-stroke ${layer.className}`}
                    d={stroke.d}
                    stroke={stroke.color}
                    style={{
                      '--xy-intro-delay': `${stroke.delay}s`,
                      '--xy-intro-duration': `${stroke.duration}s`,
                      strokeWidth: stroke[layer.widthKey],
                    } as React.CSSProperties}
                  />
                );
              }),
            )}
          </g>
        </svg>
      </div>
    </div>
  );
}
