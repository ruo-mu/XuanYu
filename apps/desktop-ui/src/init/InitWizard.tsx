/**
 * 首次安装向导：模型、安全策略、子 Agent、审阅后写出配置。
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Alert,
  Button,
  ConfigProvider,
  Divider,
  Form,
  Segmented,
  Select,
  Switch,
  Tag,
  Typography,
  message,
} from 'antd';
import enUS from 'antd/locale/en_US';
import zhCN from 'antd/locale/zh_CN';
import {
  ApiOutlined,
  ExperimentOutlined,
  GlobalOutlined,
  LockOutlined,
  MoonOutlined,
  RocketOutlined,
  SafetyOutlined,
  SunOutlined,
} from '@ant-design/icons';

import { useI18n, useLocaleText, useT } from '../i18n';
import { useTheme } from '../theme/ThemeContext';
import { DESKTOP_GATEWAY_DEFAULT } from '../foundation/config/types';
import AmbientBackground from '../components/shell/AmbientBackground';
import { BrandGlyph } from '../components/brand/BrandMark';
import { buildAntdAppTheme } from '../theme/antdAppTheme';
import type { InitDraft, InitMeta, InitProviderDef } from './initApi';
import {
  fetchInitMeta,
  hasUsableInitConfig,
  persistInitDraft,
  saveInitDraft,
  validateInitDraft,
} from './initApi';
import {
  createDraftFromSaved,
  mergeWizardSessionFields,
  normalizeInitDraftWatchdog,
} from './initDraftUtils';
import {
  renderToolPolicyTag,
  toolPolicySelectOptions,
  type Locale as ToolLocale,
} from '../utils/toolCatalog';
import ModelsStep from './ModelsStep';
import AgentsStep from './AgentsStep';
import ReviewStep from './ReviewStep';
import InitDonePanel from './InitDonePanel';
import { useScrollEdgeFade } from './useScrollEdgeFade';
import {
  clearInitWizardState,
  loadInitWizardState,
  saveInitWizardState,
} from './initDraftStorage';
import './initWizard.css';

const { Title } = Typography;

const STEP_KEYS = [
  { key: 'welcome', icon: <RocketOutlined /> },
  { key: 'models', icon: <ApiOutlined /> },
  { key: 'security', icon: <LockOutlined /> },
  { key: 'agents', icon: <ExperimentOutlined /> },
  { key: 'review', icon: <SafetyOutlined /> },
] as const;

const STEP_DESC: Partial<Record<(typeof STEP_KEYS)[number]['key'], string>> = {
  welcome: 'init.welcome.desc',
  models: 'init.models.desc',
  agents: 'init.agents.desc',
  review: 'init.review.desc',
};

/** 被谁调用：InitWizard。做什么：创建对象或草稿。 */
function createInitialDraft(meta: InitMeta): InitDraft {
  const first = meta.providers[0];
  const plan = first?.plans[0];
  const locale = meta.locale === 'en' ? 'en' : 'zh-CN';
  return {
    locale,
    providerPlanId: plan?.id,
    config: {
      ...meta.defaults,
      locale,
      provider: {
        ...meta.defaults.provider,
        type: first?.type || 'openai',
        model: meta.defaults.provider?.model || '',
        baseUrl: plan?.baseUrl,
      },
      // 通信方式写死：桌面端走 Electron IPC，不暴露网关配置。
      gateway: { ...DESKTOP_GATEWAY_DEFAULT },
    },
  };
}

/** 配置向导：主程序内的一个页面，包一层独立主题后进多步向导。 */
export default function InitWizard({ onExit }: { onExit: () => void }) {
  const { locale, setLocale } = useI18n();
  const tr = useT();
  const { mode, setMode, antdTokens, resolved } = useTheme();
  const t = resolved.tokens;
  const isDark = mode === 'dark';
  const antdLocale = locale === 'en' ? enUS : zhCN;

  const stepItems = useMemo(
    () =>
      STEP_KEYS.map((item) => ({
        ...item,
        title: tr(`init.steps.${item.key}`),
      })),
    [tr],
  );

  const wizardTheme = useMemo(
    () => buildAntdAppTheme({ antdTokens, tokens: t, isDark }, 'init'),
    [antdTokens, isDark, t],
  );
  const [meta, setMeta] = useState<InitMeta | null>(null);
  const [draft, setDraft] = useState<InitDraft | null>(null);
  const [step, setStep] = useState(0);
  const [navDirection, setNavDirection] = useState<'forward' | 'back'>('forward');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [validationErrors, setValidationErrors] = useState<string[]>([]);
  const [saveLogs, setSaveLogs] = useState<string[]>([]);
  const [done, setDone] = useState<{ url?: string; token?: string; path: string; gatewayStarted?: boolean } | null>(null);
  const draftInitialized = useRef(false);
  const loadedConfigPathRef = useRef<string>();
  const persistTimerRef = useRef<ReturnType<typeof setTimeout>>();

  const initLocale = locale === 'en' ? 'en' : 'zh-CN';
  const goToStep = useCallback((nextStep: number | ((current: number) => number)) => {
    setStep((current) => {
      const raw = typeof nextStep === 'function' ? nextStep(current) : nextStep;
      const next = Math.max(0, Math.min(stepItems.length - 1, raw));
      setNavDirection(next >= current ? 'forward' : 'back');
      return next;
    });
  }, [stepItems.length]);
  const { scrollRef, edgeClass } = useScrollEdgeFade(step);

  const onLocaleChange = useCallback(
    (l: 'zh-CN' | 'en') => {
      setLocale(l);
      setDraft((prev) =>
        prev ? { ...prev, locale: l, config: { ...prev.config, locale: l } } : prev,
      );
    },
    [setLocale],
  );

  const onThemeModeChange = useCallback(
    (m: 'light' | 'dark') => {
      setMode(m);
      setDraft((prev) => {
        if (!prev) return prev;
        const theme = (prev.config.theme || {}) as Record<string, unknown>;
        return {
          ...prev,
          config: {
            ...prev.config,
            theme: { ...theme, codeTheme: m },
          },
        };
      });
    },
    [setMode],
  );

  useEffect(() => {
    if (done) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.altKey || event.ctrlKey || event.metaKey) return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      if (tag === 'input' || tag === 'textarea' || target?.isContentEditable) return;
      if (event.key === 'ArrowLeft') goToStep((s) => s - 1);
      if (event.key === 'ArrowRight') goToStep((s) => s + 1);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [done, goToStep]);

  useEffect(() => {
    const isFirst = !draftInitialized.current;
    if (isFirst) setLoading(true);
    void fetchInitMeta(initLocale as 'zh-CN' | 'en')
      .then((m) => {
        setMeta(m);
        if (isFirst) {
          const persisted = loadInitWizardState();
          let initialDraft: InitDraft;
          if (m.savedConfig) {
            const fromFile = createDraftFromSaved(m, m.savedConfig);
            initialDraft = normalizeInitDraftWatchdog(mergeWizardSessionFields(fromFile, persisted?.draft));
            if (persisted) {
              setStep(Math.min(persisted.step, STEP_KEYS.length - 1));
            }
            loadedConfigPathRef.current = m.configPath;
          } else if (persisted) {
            initialDraft = normalizeInitDraftWatchdog(persisted.draft);
            setStep(Math.min(persisted.step, STEP_KEYS.length - 1));
          } else {
            initialDraft = normalizeInitDraftWatchdog(createInitialDraft(m));
          }
          setDraft(initialDraft);
          setLocale(initialDraft.locale);
          draftInitialized.current = true;
        }
      })
      .catch(() => message.error(tr('init.connectFailed')))
      .finally(() => {
        if (isFirst) setLoading(false);
      });
  }, [initLocale, setLocale, tr]);

  useEffect(() => {
    if (!draftInitialized.current || !meta) return;
    if (!meta.configExists || !meta.savedConfig) {
      loadedConfigPathRef.current = meta.configPath;
      return;
    }
    if (loadedConfigPathRef.current === meta.configPath) return;
    loadedConfigPathRef.current = meta.configPath;
    setDraft((prev) => {
      if (!prev) return prev;
      const fromFile = createDraftFromSaved(meta, meta.savedConfig!);
      return mergeWizardSessionFields(fromFile, prev);
    });
  }, [meta]);

  useEffect(() => {
    if (!draft || done || !draftInitialized.current) return;
    saveInitWizardState({ draft, step });
    // 草稿不完整（provider 有 type 却缺 model）时不能落盘：引擎会拒绝非法配置。
    if (!hasUsableInitConfig(draft.config)) return;
    clearTimeout(persistTimerRef.current);
    persistTimerRef.current = setTimeout(() => {
      void persistInitDraft(draft)
        .then((result) => {
          if (!result.ok) {
            message.warning(result.error || tr('init.persistFailed'));
            return;
          }
          setMeta((prev) =>
            prev
              ? { ...prev, configPath: result.configPath, configExists: true }
              : prev,
          );
        })
        .catch(() => message.warning(tr('init.persistFailed')));
    }, 400);
    return () => clearTimeout(persistTimerRef.current);
  }, [draft, step, done, tr]);

  const providerDef = useMemo(() => {
    if (!meta || !draft) return null;
    return meta.providers.find((p) => p.type === draft.config.provider?.type) || null;
  }, [meta, draft]);

  const patchConfig = useCallback((partial: InitDraft['config']) => {
    setDraft((prev) => (prev ? { ...prev, config: { ...prev.config, ...partial } } : prev));
  }, []);

  const onProviderChange = (type: string) => {
    if (!meta) return;
    const def = meta.providers.find((p) => p.type === type);
    if (!def) return;
    const plan = def.plans[0];
    setDraft((prev) =>
      prev
        ? {
            ...prev,
            providerPlanId: plan?.id,
            config: {
              ...prev.config,
              provider: {
                ...prev.config.provider!,
                type: def.type,
                model: '',
                baseUrl: plan?.baseUrl,
              },
            },
          }
        : prev,
    );
  };

  const onSave = async () => {
    if (!draft) return;
    setSaving(true);
    setValidationErrors([]);
    try {
      const v = await validateInitDraft(draft.config);
      if (!v.ok) {
        setValidationErrors(v.errors);
        message.error(tr('init.validationFailedSave'));
        goToStep(stepItems.length - 1);
        return;
      }
      const result = await saveInitDraft(draft);
      setSaveLogs(result.logs || []);
      if (!result.ok) {
        setValidationErrors(result.errors || [tr('init.saveFailed')]);
        return;
      }
      setDone({
        url: result.gatewayUrl,
        token: result.gatewayToken,
        path: result.configPath,
        gatewayStarted: result.gatewayStarted,
      });
      clearInitWizardState();
      message.success(tr('init.saveOk'));
    } catch (err) {
      message.error(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  if (loading || !draft || !meta) {
    return (
      <ConfigProvider locale={antdLocale} theme={wizardTheme}>
        <div className="xy-init-shell">
          <AmbientBackground />
          <div className="xy-init-loading">{loading ? tr('init.loading') : tr('init.loadFailed')}</div>
        </div>
      </ConfigProvider>
    );
  }

  if (done) {
    return (
      <ConfigProvider locale={antdLocale} theme={wizardTheme}>
        <div className="xy-init-shell">
          <AmbientBackground />
          <InitDonePanel
            path={done.path}
            url={done.url}
            token={done.token}
            gatewayStarted={done.gatewayStarted}
            onDone={onExit}
          />
        </div>
      </ConfigProvider>
    );
  }

  const currentStep = stepItems[step] || stepItems[0];
  const progress = Math.round(((step + 1) / stepItems.length) * 100);
  const stepDescKey = STEP_DESC[currentStep.key as keyof typeof STEP_DESC];

  return (
    <ConfigProvider locale={antdLocale} theme={wizardTheme}>
      <div className="xy-init-shell">
        <AmbientBackground />
        <div className="xy-init-workspace">
          <aside className="xy-init-rail" aria-label={tr('init.progressAria')}>
            <div className="xy-init-rail-brand">
              <BrandGlyph size={34} ariaLabel={tr('init.brandTitle')} />
              <span>{tr('init.brandSub')}</span>
              <strong>{tr('init.brandTitle')}</strong>
            </div>
            <div className="xy-init-rail-progress">
              <span>{String(step + 1).padStart(2, '0')}</span>
              <div><i style={{ width: `${progress}%` }} /></div>
              <span>{String(stepItems.length).padStart(2, '0')}</span>
            </div>
            <div className="xy-init-rail-steps">
              {stepItems.map((item, index) => (
                <button
                  key={item.key}
                  type="button"
                  className={`xy-init-rail-step ${index === step ? 'is-active' : ''} ${index < step ? 'is-done' : ''}`}
                  onClick={() => goToStep(index)}
                  aria-current={index === step ? 'step' : undefined}
                >
                  <span className="xy-init-rail-step__icon">{item.icon}</span>
                  <strong>{item.title}</strong>
                </button>
              ))}
            </div>
          </aside>

          <main className="xy-init-main">
            <section className="xy-init-stage" aria-live="polite">
              <div className={`xy-init-page is-${navDirection}`} key={currentStep.key}>
                <header className="xy-init-page-heading">
                  <div className="xy-init-page-heading__main">
                    <div
                      className="xy-init-step-badge"
                      aria-label={tr('init.stepOf', undefined, step + 1, stepItems.length)}
                    >
                      <span className="xy-init-step-badge__label">{tr('init.stepLabel')}</span>
                      <span className="xy-init-step-badge__current">{String(step + 1).padStart(2, '0')}</span>
                      <span className="xy-init-step-badge__total">{tr('init.stepTotalOf', undefined, stepItems.length)}</span>
                    </div>
                    <div className="xy-init-page-heading__copy">
                      <h1>{currentStep.title}</h1>
                      {stepDescKey && <p className="xy-init-page-desc">{tr(stepDescKey)}</p>}
                    </div>
                  </div>
                  {meta.configExists && step > 0 && (
                    <Tag color="orange">{tr('init.overwriteConfig')}</Tag>
                  )}
                </header>

                <div className={`xy-init-page-body ${edgeClass}`.trim()}>
                  <div className="xy-init-page-scroll" ref={scrollRef}>
                    <StepPanel
                      step={currentStep.key}
                      meta={meta}
                      draft={draft}
                      providerDef={providerDef}
                      locale={locale}
                      onLocale={onLocaleChange}
                      themeMode={mode}
                      onThemeMode={onThemeModeChange}
                      patchConfig={patchConfig}
                      setDraft={setDraft}
                      onProviderChange={onProviderChange}
                    />
                    {validationErrors.length > 0 && (
                      <Alert
                        type="error"
                        showIcon
                        message={tr('init.validationFailed')}
                        description={<ul>{validationErrors.map((e) => <li key={e}>{e}</li>)}</ul>}
                        style={{ marginTop: 16 }}
                      />
                    )}
                  </div>
                </div>
              </div>
            </section>

            <div className={`xy-init-actions${step === 0 ? ' is-first-step' : ''}`}>
              {/* 向导是主程序里的一个页面，必须留一条回到工作台的出口。 */}
              <Button type="text" className="xy-init-actions__exit" onClick={onExit}>
                {tr('init.exitToApp', '返回主界面')}
              </Button>
              {step > 0 && (
                <Button disabled={saving} onClick={() => goToStep((s) => s - 1)}>{tr('init.prev')}</Button>
              )}
              {step < stepItems.length - 1 ? (
                <Button type="primary" disabled={saving} onClick={() => goToStep((s) => s + 1)}>{tr('init.next')}</Button>
              ) : (
                <Button type="primary" loading={saving} onClick={() => void onSave()}>{tr('init.save')}</Button>
              )}
            </div>
          </main>
        </div>
      </div>
    </ConfigProvider>
  );
}

interface StepPanelProps {
  step: string;
  meta: InitMeta;
  draft: InitDraft;
  providerDef: InitProviderDef | null;
  locale: string;
  themeMode: 'light' | 'dark';
  onLocale: (l: 'zh-CN' | 'en') => void;
  onThemeMode: (mode: 'light' | 'dark') => void;
  patchConfig: (p: InitDraft['config']) => void;
  setDraft: React.Dispatch<React.SetStateAction<InitDraft | null>>;
  onProviderChange: (type: string) => void;
}

/** 欢迎步：语言、主题。 */
function WelcomePreferences({
  locale,
  themeMode,
  onLocale,
  onThemeMode,
}: {
  locale: string;
  themeMode: 'light' | 'dark';
  onLocale: (l: 'zh-CN' | 'en') => void;
  onThemeMode: (mode: 'light' | 'dark') => void;
}) {
  const tr = useT();
  const isDark = themeMode === 'dark';
  const isEnglish = locale === 'en';

  return (
    <div className="xy-init-pref-list">
      <div className="xy-init-pref-row">
        <div className="xy-init-pref-icon" aria-hidden>
          <GlobalOutlined />
        </div>
        <div className="xy-init-pref-copy">
          <strong>{tr('init.welcome.langTitle')}</strong>
          <span>{tr('init.welcome.langDesc')}</span>
        </div>
        <div className="xy-init-pref-control">
          <Segmented
            aria-label={tr('init.welcome.langAria')}
            value={isEnglish ? 'en' : 'zh-CN'}
            onChange={(value) => onLocale(value as 'zh-CN' | 'en')}
            options={[
              { label: tr('init.welcome.langZh'), value: 'zh-CN' },
              { label: tr('init.welcome.langEn'), value: 'en' },
            ]}
          />
        </div>
      </div>

      <div className="xy-init-pref-row">
        <div className="xy-init-pref-icon" aria-hidden>
          {isDark ? <MoonOutlined /> : <SunOutlined />}
        </div>
        <div className="xy-init-pref-copy">
          <strong>{tr('init.welcome.themeTitle')}</strong>
          <span>{isDark ? tr('init.welcome.themeDark') : tr('init.welcome.themeLight')}</span>
        </div>
        <div className="xy-init-pref-control">
          <Segmented
            aria-label={tr('init.welcome.themeAria')}
            value={themeMode}
            onChange={(value) => onThemeMode(value as 'light' | 'dark')}
            options={[
              { value: 'light', label: tr('header.theme.light'), icon: <SunOutlined /> },
              { value: 'dark', label: tr('header.theme.dark'), icon: <MoonOutlined /> },
            ]}
          />
        </div>
      </div>
    </div>
  );
}

/** 按当前 step 渲染模型/频道/工具/回顾等面板。 */
function StepPanel(props: StepPanelProps) {
  const { step, meta, draft, providerDef, patchConfig, setDraft, onProviderChange } = props;
  const tr = useT();
  const lt = useLocaleText();
  const toolLocale: ToolLocale = props.locale === 'en' ? 'en' : 'zh-CN';
  const toolPolicyOptions = toolPolicySelectOptions(toolLocale);
  const p = draft.config.provider!;

  switch (step) {
    case 'welcome':
      return (
        <WelcomePreferences
          locale={props.locale}
          themeMode={props.themeMode}
          onLocale={props.onLocale}
          onThemeMode={props.onThemeMode}
        />
      );

    case 'models':
      return (
        <ModelsStep
          meta={meta}
          draft={draft}
          providerDef={providerDef}
          patchConfig={patchConfig}
          setDraft={setDraft}
          onProviderChange={onProviderChange}
        />
      );

    case 'security': {
      const permissionsBase = {
        ...meta.defaults.permissions,
        ...draft.config.permissions,
      };
      const sandboxBase = {
        enabled: true,
        cwdAllowlist: [] as string[],
        autoForRiskyCommands: true,
        ...meta.defaults.sandbox,
        ...draft.config.sandbox,
      };
      const patchSandbox = (partial: Partial<typeof sandboxBase>) => {
        patchConfig({
          sandbox: { ...sandboxBase, enabled: true, autoForRiskyCommands: true, ...partial },
        });
      };
      const patchPermissions = (partial: Partial<typeof permissionsBase>) => {
        patchConfig({ permissions: { ...permissionsBase, ...partial } });
      };
      return (
        <>
          <Title level={4}>{tr('init.security.title')}</Title>
          <Form layout="vertical">
            <Divider>{tr('init.security.toolsTitle')}</Divider>
            <Form.Item label={tr('init.security.toolWhitelist')}>
              <Select
                mode="tags"
                allowClear
                showSearch
                optionFilterProp="label"
                value={(permissionsBase.whitelist as string[] | undefined) || []}
                onChange={(whitelist) => patchPermissions({ whitelist: whitelist ?? [] })}
                options={toolPolicyOptions}
                placeholder={tr('init.security.toolPlaceholder')}
                tagRender={(tagProps) => renderToolPolicyTag(tagProps, toolLocale)}
              />
            </Form.Item>
            <Form.Item label={tr('init.security.disabledTools')}>
              <Select
                mode="tags"
                allowClear
                showSearch
                optionFilterProp="label"
                value={draft.config.disabledTools || []}
                onChange={(disabledTools) => patchConfig({ disabledTools: disabledTools ?? [] })}
                options={toolPolicyOptions}
                placeholder={tr('init.security.toolPlaceholder')}
                tagRender={(tagProps) => renderToolPolicyTag(tagProps, toolLocale)}
              />
            </Form.Item>
            <Divider>{tr('init.security.sandboxTitle')}</Divider>
            <Form.Item label={tr('init.security.sandboxCwdAllowlist')}>
              <Select
                mode="tags"
                allowClear
                open={false}
                value={sandboxBase.cwdAllowlist || []}
                onChange={(cwdAllowlist) => patchSandbox({ cwdAllowlist: cwdAllowlist ?? [] })}
                placeholder={tr('init.security.sandboxCwdPlaceholder')}
                tokenSeparators={[',', '，']}
              />
            </Form.Item>
          </Form>
        </>
      );
    }

    case 'agents':
      return (
        <AgentsStep
          meta={meta}
          draft={draft}
          locale={props.locale === 'en' ? 'en' : 'zh-CN'}
          patchConfig={patchConfig}
        />
      );

    case 'review':
      return (
        <ReviewStep draft={draft} meta={meta} locale={props.locale === 'en' ? 'en' : 'zh-CN'} />
      );

    default:
      return null;
  }
}
