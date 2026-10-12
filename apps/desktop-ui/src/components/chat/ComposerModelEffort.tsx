/**
 * Combined model + deep-thinking + reasoning effort control.
 * Main glass menu → model submenu; effort slider only when deep thinking is on.
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { Popover, Switch } from 'antd';
import {
  CheckOutlined,
  LoadingOutlined,
  QuestionCircleOutlined,
  ReloadOutlined,
  RightOutlined,
} from '@ant-design/icons';

import { useI18n } from '../../i18n';
import { useTheme } from '../../theme/ThemeContext';
import { isPageVisible, onPageVisibilityChange } from '../../utils/pageVisibility';

export type ReasoningEffort = 'low' | 'medium' | 'high' | 'max' | 'ultracode';

export const REASONING_EFFORTS: ReasoningEffort[] = [
  'low',
  'medium',
  'high',
  'max',
  'ultracode',
];

/** 子 Agent 深度思考滑块：最左关闭，其后为低→最强。 */
export const AGENT_THINKING_STEPS = ['off', ...REASONING_EFFORTS] as const;
export type AgentThinkingStep = (typeof AGENT_THINKING_STEPS)[number];

/** 深度思考滑块位置：关闭=0，其后对应 REASONING_EFFORTS。 */
export function agentThinkingIndex(deepThinking: boolean, effort?: ReasoningEffort): number {
  if (!deepThinking) return 0;
  const idx = REASONING_EFFORTS.indexOf(effort || 'high');
  return idx >= 0 ? idx + 1 : REASONING_EFFORTS.indexOf('high') + 1;
}

/** 滑块下标 → deepThinking + reasoningEffort，写回会话配置。 */
export function agentThinkingFromIndex(index: number): {
  deepThinking: boolean;
  reasoningEffort?: ReasoningEffort;
} {
  const clamped = Math.max(0, Math.min(AGENT_THINKING_STEPS.length - 1, index));
  if (clamped === 0) return { deepThinking: false };
  return {
    deepThinking: true,
    reasoningEffort: REASONING_EFFORTS[clamped - 1] || 'high',
  };
}

/**
 * Ultracode fill: theme primary as a solid wash, masked so opacity is exactly
 * left 0.2 → right 1.0 (linear). Base stays the normal gray rail underneath —
 * otherwise a solid colored base hides the fade (everything looks equally deep).
 */
function ultracodeFillStyle(primary: string): React.CSSProperties {
  const fade = 'linear-gradient(90deg, rgba(0,0,0,0.2) 0%, rgba(0,0,0,1) 100%)';
  return {
    background: primary,
    WebkitMaskImage: fade,
    maskImage: fade,
    boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.2)',
  };
}

interface ComposerModelEffortProps {
  model: string;
  models: string[];
  modelsLoading?: boolean;
  onRefreshModels?: () => void;
  effort: ReasoningEffort;
  deepThinking: boolean;
  onModelChange: (model: string) => void;
  onEffortChange: (effort: ReasoningEffort) => void;
  onDeepThinkingChange: (enabled: boolean) => void;
  onOpenChange?: (open: boolean) => void;
}

/** 被谁调用：ChatView composer。做什么：选模型与 reasoning effort。 */
export default function ComposerModelEffort({
  model,
  models,
  modelsLoading,
  onRefreshModels,
  effort,
  deepThinking,
  onModelChange,
  onEffortChange,
  onDeepThinkingChange,
  onOpenChange,
}: ComposerModelEffortProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);

  useEffect(() => {
    onOpenChange?.(open || modelOpen);
  }, [open, modelOpen, onOpenChange]);
  const effortIndex = Math.max(0, REASONING_EFFORTS.indexOf(effort));
  const effortLabel = t(`chat.reasoning.${effort}`);
  const isUltracode = deepThinking && effort === 'ultracode';

  const modelList = useMemo(() => {
    const set = new Set<string>();
    if (model) set.add(model);
    for (const name of models) if (name) set.add(name);
    return Array.from(set);
  }, [models, model]);

  const displayModel = model || t('chat.modelSelect');
  const triggerText = deepThinking ? `${displayModel} ${effortLabel}` : displayModel;

  const mainPanel = (
    <div className="xy-me-panel">
      <Popover
        open={modelOpen}
        onOpenChange={setModelOpen}
        trigger="click"
        placement="rightTop"
        arrow={false}
        align={{ offset: [12, 0] }}
        autoAdjustOverflow={{ adjustX: 1, adjustY: 1 }}
        getPopupContainer={() => document.body}
        overlayClassName="xy-me-glass-overlay xy-me-model-overlay"
        destroyTooltipOnHide
        content={
          <div className="xy-me-panel xy-me-panel-models">
            <div className="xy-me-panel-caption">
              <span>{t('chat.model')}</span>
              {onRefreshModels ? (
                <button
                  type="button"
                  className="xy-me-model-refresh"
                  onClick={(e) => {
                    e.stopPropagation();
                    onRefreshModels();
                  }}
                  disabled={modelsLoading}
                  aria-label={t('chat.modelRefresh')}
                  title={t('chat.modelRefresh')}
                >
                  {modelsLoading ? <LoadingOutlined /> : <ReloadOutlined />}
                </button>
              ) : null}
            </div>
            <div className="xy-me-model-list" role="listbox" aria-label={t('chat.model')}>
              {modelList.map((name) => {
                const selected = name === model;
                return (
                  <button
                    key={name}
                    type="button"
                    role="option"
                    aria-selected={selected}
                    className={`xy-me-model-item${selected ? ' is-selected' : ''}`}
                    onClick={() => {
                      onModelChange(name);
                      setModelOpen(false);
                    }}
                  >
                    <span className="xy-me-model-item-label">{name}</span>
                    {selected ? <CheckOutlined className="xy-me-model-check" /> : null}
                  </button>
                );
              })}
              {modelsLoading && modelList.length === 0 ? (
                <div className="xy-me-model-empty">{t('chat.modelLoading')}</div>
              ) : null}
              {!modelsLoading && modelList.length === 0 ? (
                <div className="xy-me-model-empty">{t('chat.modelEmpty')}</div>
              ) : null}
            </div>
          </div>
        }
      >
        <button type="button" className="xy-me-row">
          <span className="xy-me-row-label">{t('chat.model')}</span>
          <span className="xy-me-row-value">
            <span className="xy-me-row-value-text">{displayModel}</span>
            <RightOutlined className="xy-me-row-chevron" />
          </span>
        </button>
      </Popover>

      <div className="xy-me-row is-static">
        <span className="xy-me-row-label">{t('chat.deepThinking')}</span>
        <Switch
          size="small"
          checked={deepThinking}
          onChange={onDeepThinkingChange}
          aria-label={t('chat.deepThinking')}
        />
      </div>

      {deepThinking ? (
        <div className={`xy-me-effort-block${isUltracode ? ' is-ultracode' : ''}`}>
          <div className="xy-me-effort-head">
            <div className="xy-me-effort-title">
              <span className="xy-me-effort-k">{t('chat.effort')}</span>
              <span className={`xy-me-effort-v${isUltracode ? ' is-ultracode' : ''}`}>
                {effortLabel}
              </span>
            </div>
            <span className="xy-me-effort-help" title={t('chat.effortHelp')}>
              <QuestionCircleOutlined />
            </span>
          </div>
          <div className="xy-effort-slider-labels">
            <span>{t('chat.effortFaster')}</span>
            <span>{t('chat.effortSmarter')}</span>
          </div>
          <EffortSlider
            value={effortIndex}
            max={REASONING_EFFORTS.length - 1}
            isUltracode={isUltracode}
            onChange={(index) => onEffortChange(REASONING_EFFORTS[index] || 'low')}
          />
        </div>
      ) : null}
    </div>
  );

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) setModelOpen(false);
      }}
      trigger="click"
      placement="topRight"
      arrow={false}
      getPopupContainer={() => document.body}
      overlayClassName="xy-me-glass-overlay"
      destroyTooltipOnHide
      content={mainPanel}
    >
      <button
        type="button"
        className={`xy-me-trigger${deepThinking ? ' is-thinking' : ''}${isUltracode ? ' is-ultracode' : ''}`}
        aria-label={t('chat.modelEffortMenu')}
      >
        <span className="xy-me-trigger-label">{triggerText}</span>
      </button>
    </Popover>
  );
}

/** 被谁调用：ComposerModelEffort。做什么：推理力度滑块。 */
export function EffortSlider({
  value,
  max,
  isUltracode,
  onChange,
  className,
}: {
  value: number;
  max: number;
  isUltracode?: boolean;
  onChange: (index: number) => void;
  className?: string;
}) {
  const trackRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  const showStarfield = !!isUltracode;
  // Same source as Switch: antdTokens.colorPrimary === resolved.tokens.primary
  const { antdTokens } = useTheme();
  const primary = antdTokens.colorPrimary;
  const fillExtra = useMemo(
    () => (isUltracode ? ultracodeFillStyle(primary) : null),
    [isUltracode, primary],
  );

  const indexFromClientX = useCallback(
    (clientX: number) => {
      const el = trackRef.current;
      if (!el) return value;
      const rect = el.getBoundingClientRect();
      const ratio = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
      return Math.round(ratio * max);
    },
    [max, value],
  );

  const setFromEvent = useCallback(
    (clientX: number) => {
      onChange(indexFromClientX(clientX));
    },
    [indexFromClientX, onChange],
  );

  const onPointerDown = (e: React.PointerEvent) => {
    dragging.current = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    setFromEvent(e.clientX);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (!dragging.current) return;
    setFromEvent(e.clientX);
  };

  const onPointerUp = (e: React.PointerEvent) => {
    dragging.current = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      // ignore
    }
  };

  const pct = max <= 0 ? 0 : (value / max) * 100;
  const progress = pct / 100;
  // 16px thumb in 20px track → 2px inset; center runs 10px … width-10px.
  // Off (progress=0): no fill under the semi-transparent dark-theme thumb,
  // otherwise the 16×20 pill reads as a clipped / non-round knob.
  const thumbLeft = `calc(10px + (100% - 20px) * ${progress})`;
  const fillWidth =
    progress <= 0 ? '0px' : `calc(10px + (100% - 20px) * ${progress} + 8px)`;

  return (
    <div
      className={`xy-effort-slider${isUltracode ? ' is-ultracode' : ''}${className ? ` ${className}` : ''}`}
      role="slider"
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={value}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') {
          e.preventDefault();
          onChange(Math.max(0, value - 1));
        }
        if (e.key === 'ArrowRight' || e.key === 'ArrowUp') {
          e.preventDefault();
          onChange(Math.min(max, value + 1));
        }
      }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <div ref={trackRef} className="xy-effort-track">
        <div className="xy-effort-track-surface" aria-hidden>
          <div className="xy-effort-track-base" />
          <div
            className="xy-effort-track-fill"
            style={{ width: fillWidth, ...fillExtra }}
          />
          {showStarfield ? <EffortStarfield progress={progress} primary={primary} /> : null}
          <div className="xy-effort-dots">
            {Array.from({ length: max + 1 }, (_, i) => (
              <span
                key={i}
                className={`xy-effort-dot${i === value ? ' is-active' : ''}${i === max ? ' is-tip' : ''}`}
                style={{ left: `calc(10px + (100% - 20px) * ${i / Math.max(1, max)})` }}
              />
            ))}
          </div>
        </div>
        <div className="xy-effort-thumb" style={{ left: thumbLeft }} aria-hidden />
      </div>
    </div>
  );
}

/**
 * Ultracode-only flowing star stream.
 * Emit from thumb → drift left; right dense/opaque, left sparse/transparent.
 */
function EffortStarfield({ progress, primary }: { progress: number; primary: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const progressRef = useRef(progress);
  progressRef.current = progress;
  const primaryRef = useRef(primary);
  primaryRef.current = primary;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let raf = 0;
    let running = true;
    let last = performance.now();
    let spawnAcc = 0;

    type Star = {
      x: number;
      y: number;
      size: number;
      speed: number;
      wobble: number;
      phase: number;
      birth: number;
      life: number;
      brightness: number;
    };
    const stars: Star[] = [];

    const resize = () => {
      const parent = canvas.parentElement;
      if (!parent) return;
      const rect = parent.getBoundingClientRect();
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      canvas.width = Math.max(1, Math.floor(rect.width * dpr));
      canvas.height = Math.max(1, Math.floor(rect.height * dpr));
      canvas.style.width = `${rect.width}px`;
      canvas.style.height = `${rect.height}px`;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };

    resize();
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(resize) : null;
    if (canvas.parentElement && ro) ro.observe(canvas.parentElement);

    const thumbX = (w: number) => {
      const p = Math.max(0, Math.min(1, progressRef.current));
      return 8 + (w - 16) * p;
    };

    const spawn = (w: number, h: number, now: number) => {
      if (stars.length >= 220) return;
      const origin = thumbX(w);
      const x = origin - 4 + Math.random() * 8;
      stars.push({
        x: Math.min(w - 2, Math.max(2, x)),
        y: h * (0.18 + Math.random() * 0.64),
        size: 0.55 + Math.random() * 1.45,
        speed: 28 + Math.random() * 48,
        wobble: (Math.random() - 0.5) * 7,
        phase: Math.random() * Math.PI * 2,
        birth: now,
        life: 1.1 + Math.random() * 1.4,
        brightness: 0.55 + Math.random() * 0.45,
      });
    };

    {
      const w = canvas.clientWidth || 240;
      const h = canvas.clientHeight || 20;
      const now = performance.now();
      const origin = thumbX(w);
      for (let i = 0; i < 48; i++) {
        spawn(w, h, now);
        const s = stars[stars.length - 1];
        if (s) s.x -= Math.random() * Math.max(8, origin * 0.92);
      }
    }

    const tick = (now: number) => {
      if (!running) return;
      if (!isPageVisible()) {
        raf = 0;
        return;
      }
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;

      const w = canvas.clientWidth || 1;
      const h = canvas.clientHeight || 1;
      const origin = thumbX(w);
      const streamLen = Math.max(80, origin * 0.96);
      const themePrimary = primaryRef.current;

      spawnAcc += 92 * dt;
      while (spawnAcc >= 1) {
        spawn(w, h, now);
        spawnAcc -= 1;
      }

      ctx.clearRect(0, 0, w, h);

      for (let i = stars.length - 1; i >= 0; i--) {
        const s = stars[i];
        s.x -= s.speed * dt;
        s.y += Math.sin(now / 220 + s.phase) * s.wobble * dt * 0.3;

        const age = (now - s.birth) / 1000;
        const dist = streamLen > 0 ? (origin - s.x) / streamLen : 0;
        if (age >= s.life || s.x < -4 || dist > 1.05) {
          stars.splice(i, 1);
          continue;
        }

        const lifeIn = age < 0.1 ? age / 0.1 : 1;
        const lifeOut = age > s.life * 0.7 ? Math.max(0, (s.life - age) / (s.life * 0.3)) : 1;
        const twinkle = 0.82 + 0.18 * Math.sin(now / 130 + s.phase * 1.6);
        const alpha = Math.max(0, Math.min(1, lifeIn * lifeOut * twinkle * s.brightness));
        if (alpha < 0.04) continue;

        const trail = Math.min(12, 4 + s.speed * 0.07);
        const streak = ctx.createLinearGradient(s.x, s.y, s.x + trail, s.y);
        streak.addColorStop(0, `rgba(255, 255, 255, ${alpha})`);
        streak.addColorStop(
          1,
          themePrimary.startsWith('#') && themePrimary.length === 7
            ? `${themePrimary}00`
            : 'rgba(255,255,255,0)',
        );
        ctx.strokeStyle = streak;
        ctx.lineWidth = Math.max(0.55, s.size * 0.5);
        ctx.beginPath();
        ctx.moveTo(s.x, s.y);
        ctx.lineTo(s.x + trail, s.y);
        ctx.stroke();

        ctx.beginPath();
        ctx.fillStyle = `rgba(255, 255, 255, ${alpha})`;
        ctx.arc(s.x, s.y, s.size * 1.05, 0, Math.PI * 2);
        ctx.fill();

        if (alpha > 0.55) {
          ctx.beginPath();
          ctx.fillStyle = `rgba(255, 255, 255, ${alpha * 0.2})`;
          ctx.arc(s.x, s.y, s.size * 2.1, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      raf = requestAnimationFrame(tick);
    };

    raf = requestAnimationFrame(tick);
    const unsubVisibility = onPageVisibilityChange(() => {
      if (!running) return;
      if (isPageVisible() && !raf) {
        last = performance.now();
        raf = requestAnimationFrame(tick);
      }
    });

    return () => {
      running = false;
      cancelAnimationFrame(raf);
      unsubVisibility();
      ro?.disconnect();
    };
  }, []);

  return <canvas className="xy-effort-starfield" ref={canvasRef} aria-hidden />;
}
