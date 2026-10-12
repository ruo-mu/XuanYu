/**
 * 初始化向导「审阅」步：汇总模型、安全策略与子 Agent 后再写入。
 */

import React, { useMemo } from 'react';
import { ApiOutlined, LockOutlined, RobotOutlined, SafetyOutlined } from '@ant-design/icons';
import { Tag, Typography } from 'antd';

import { useT } from '../i18n';
import { displayFsPath } from '../foundation/config/paths';
import { subAgentRoleLabel } from '../utils/subagentRoles';
import { INIT_WATCHDOG_DEFAULTS, normalizeWatchdogForInit } from './initDraftUtils';
import type { InitDraft, InitMeta } from './initApi';
import {
  formatModelLine,
  isAuxInherited,
  MODEL_SLOTS,
  type AgentModelPatch,
  type AuxModelTask,
} from './modelSlots';

const { Text } = Typography;

const BUILTIN_AGENT_IDS = ['architect', 'developer', 'explorer', 'reviewer', 'doc-writer'] as const;

type BuiltinAgentId = (typeof BUILTIN_AGENT_IDS)[number];

interface ReviewStepProps {
  draft: InitDraft;
  meta: InitMeta;
  locale: 'zh-CN' | 'en';
}

/** 被谁调用：ReviewStep。做什么：一块复核卡片容器。 */
function ReviewCard({
  icon,
  title,
  children,
  className,
}: {
  icon: React.ReactNode;
  title: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <article className={`xy-init-review-card${className ? ` ${className}` : ''}`}>
      <header className="xy-init-review-card__head">
        <span className="xy-init-review-card__icon" aria-hidden>
          {icon}
        </span>
        <strong>{title}</strong>
      </header>
      <div className="xy-init-review-card__body">{children}</div>
    </article>
  );
}

/** 被谁调用：ReviewCard。做什么：标签+值行。 */
function ReviewRow({ label, value, mono }: { label: string; value: React.ReactNode; mono?: boolean }) {
  return (
    <div className="xy-init-review-row">
      <span className="xy-init-review-row__label">{label}</span>
      <span className={`xy-init-review-row__value${mono ? ' is-mono' : ''}`}>{value}</span>
    </div>
  );
}

/** 被谁调用：ReviewStep。做什么：内置 Agent 是否未单独配模型。 */
function isAgentModelInherited(models: Record<string, AgentModelPatch>, id: BuiltinAgentId): boolean {
  const value = models[id];
  return !value || (!value.provider && !value.model && !value.apiKey && !value.baseUrl);
}

/** 被谁调用：InitWizard。做什么：确认页汇总 draft。 */
export default function ReviewStep({ draft, meta, locale }: ReviewStepProps) {
  const tr = useT();
  const config = draft.config;
  const provider = config.provider || {};
  const auxModels = (config.modelRouting?.auxiliaryModels || {}) as Partial<
    Record<AuxModelTask, AgentModelPatch>
  >;
  const subAgents = normalizeWatchdogForInit(
    (config.subAgents || {}) as Record<string, unknown>,
  );
  const agentModels = (subAgents.models || {}) as Record<string, AgentModelPatch>;
  const permissions = config.permissions as
    | { whitelist?: string[] }
    | undefined;
  const sandbox = config.sandbox as
    | { cwdAllowlist?: string[] }
    | undefined;
  const watchdog = (subAgents.watchdog || INIT_WATCHDOG_DEFAULTS) as {
    silentMs: number;
    noProgressIterations: number;
    maxRetries: number;
  };

  const customAuxSlots = useMemo(
    () =>
      MODEL_SLOTS.filter(
        (slot) => slot.kind === 'auxiliary' && !isAuxInherited(auxModels, slot.id as AuxModelTask),
      ),
    [auxModels],
  );

  const whitelist = permissions?.whitelist?.length ?? 0;
  const disabledTools = config.disabledTools?.length ?? 0;
  const cwdAllowlist = sandbox?.cwdAllowlist?.length ?? 0;
  const sandboxRoot = tr('init.review.sandboxDefaultRoot');

  return (
    <div className="xy-init-review">
      <div className="xy-init-review-hero">
        <div className="xy-init-review-hero__glow" aria-hidden />
        <div className="xy-init-review-hero__icon" aria-hidden>
          <SafetyOutlined />
        </div>
        <div className="xy-init-review-hero__copy">
          <strong>{tr('init.review.heroTitle')}</strong>
          <p>{tr('init.review.heroDesc')}</p>
          <div className="xy-init-review-hero__paths">
            <span>
              <em>{tr('init.review.configFile')}</em>
              <code>{displayFsPath(meta.configPath)}</code>
            </span>
            <span>
              <em>{tr('init.review.dataDir')}</em>
              <code>{displayFsPath(meta.appDataDir)}</code>
            </span>
          </div>
        </div>
      </div>

      <div className="xy-init-review-grid">
        <ReviewCard icon={<ApiOutlined />} title={tr('init.review.section.models')}>
          <ReviewRow
            label={tr('init.review.mainModel')}
            value={formatModelLine(provider.type, provider.model) || '—'}
          />
          <ReviewRow
            label={tr('init.review.auxModels')}
            value={
              customAuxSlots.length
                ? tr('init.review.auxCustom', undefined, customAuxSlots.length)
                : tr('init.review.auxInheritAll')
            }
          />
          {customAuxSlots.length > 0 ? (
            <ul className="xy-init-review-list">
              {customAuxSlots.map((slot) => {
                const aux = auxModels[slot.id as AuxModelTask] as AgentModelPatch;
                return (
                  <li key={slot.id}>
                    <span>{tr(slot.nameKey)}</span>
                    <Text type="secondary">{formatModelLine(aux.provider || provider.type, aux.model)}</Text>
                  </li>
                );
              })}
            </ul>
          ) : null}
        </ReviewCard>

        <ReviewCard icon={<LockOutlined />} title={tr('init.review.section.security')}>
          <ReviewRow
            label={tr('init.security.toolWhitelist')}
            value={whitelist ? String(whitelist) : tr('init.review.none')}
          />
          <ReviewRow
            label={tr('init.security.disabledTools')}
            value={disabledTools ? String(disabledTools) : tr('init.review.none')}
          />
          <ReviewRow label={tr('init.security.sandboxRootDir')} value={sandboxRoot} mono />
          <ReviewRow
            label={tr('init.security.sandboxCwdAllowlist')}
            value={
              cwdAllowlist
                ? tr('init.review.pathCount', undefined, cwdAllowlist)
                : tr('init.review.workspaceOnly')
            }
          />
        </ReviewCard>

        <ReviewCard icon={<RobotOutlined />} title={tr('init.review.section.agents')} className="is-wide">
          <div className="xy-init-review-agent-grid">
            {BUILTIN_AGENT_IDS.map((id) => {
              const roleId = id === 'doc-writer' ? 'writer' : id;
              const inherited = isAgentModelInherited(agentModels, id);
              const modelLine = inherited
                ? tr('init.agents.inheritModelHint')
                : formatModelLine(
                    agentModels[id]?.provider || provider.type,
                    agentModels[id]?.model || provider.model,
                  );
              return (
                <div className="xy-init-review-agent" key={id}>
                  <div className="xy-init-review-agent__head">
                    <strong>{subAgentRoleLabel(roleId, locale)}</strong>
                    <Tag color={inherited ? 'green' : 'blue'}>
                      {inherited ? tr('init.models.inheritTag') : tr('init.models.customTag')}
                    </Tag>
                  </div>
                  <p className="xy-init-review-agent__model">{modelLine}</p>
                </div>
              );
            })}
          </div>
          <div className="xy-init-review-watchdog">
            <span>{tr('init.agents.watchdogTitle')}</span>
            <div className="xy-init-review-watchdog__chips">
              <Tag>{tr('init.agents.silentMs')}: {watchdog.silentMs}</Tag>
              <Tag>{tr('init.agents.noProgress')}: {watchdog.noProgressIterations}</Tag>
              <Tag>{tr('init.agents.maxRetries')}: {watchdog.maxRetries}</Tag>
            </div>
          </div>
        </ReviewCard>
      </div>

      <p className="xy-init-review-footnote">{tr('init.review.footnote')}</p>
    </div>
  );
}
