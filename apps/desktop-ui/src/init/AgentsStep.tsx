/**
 * 初始化向导「子 Agent」步：启用内置专家、编辑模型与 watchdog。
 */

import React, { useEffect, useMemo, useState } from 'react';
import {
  Button,
  Card,
  Col,
  Divider,
  Form,
  Input,
  InputNumber,
  Modal,
  Row,
  Switch,
  Tag,
  Typography,
  message,
} from 'antd';
import { EditOutlined } from '@ant-design/icons';

import { useT } from '../i18n';
import { ModelSelect, ProviderSelect } from '../components/model';
import { useProviderModels } from '../modelCatalog';
import { subAgentRoleLabel } from '../utils/subagentRoles';
import type { InitDraft, InitMeta } from './initApi';
import { formatModelLine, type AgentModelPatch } from './modelSlots';
import { INIT_WATCHDOG_DEFAULTS, normalizeWatchdogForInit } from './initDraftUtils';

const { Title, Text } = Typography;

/** 内置专家子 Agent 的 id。 */
export type BuiltinAgentId = 'architect' | 'developer' | 'explorer' | 'reviewer' | 'doc-writer';

const BUILTIN_AGENT_IDS: BuiltinAgentId[] = [
  'architect',
  'developer',
  'explorer',
  'reviewer',
  'doc-writer',
];

const AGENT_DESC_KEYS: Record<BuiltinAgentId, string> = {
  architect: 'init.agents.role.architect',
  developer: 'init.agents.role.developer',
  explorer: 'init.agents.role.explorer',
  reviewer: 'init.agents.role.reviewer',
  'doc-writer': 'init.agents.role.writer',
};

interface AgentsStepProps {
  meta: InitMeta;
  draft: InitDraft;
  locale: 'zh-CN' | 'en';
  patchConfig: (p: InitDraft['config']) => void;
}

/** 被谁调用：ReviewStep。做什么：内置 Agent 是否未单独配模型。 */
function isAgentModelInherited(
  models: Record<string, AgentModelPatch>,
  id: BuiltinAgentId,
): boolean {
  const value = models[id];
  return !value || (!value.provider && !value.model && !value.apiKey && !value.baseUrl);
}

/** 向导中配置内置子 Agent 的启用状态与模型。 */
export default function AgentsStep({ meta, draft, locale, patchConfig }: AgentsStepProps) {
  const tr = useT();
  const p = draft.config.provider!;
  const subAgents = normalizeWatchdogForInit(
    (draft.config.subAgents || {}) as Record<string, unknown>,
  );
  const watchdog = (subAgents.watchdog || INIT_WATCHDOG_DEFAULTS) as {
    silentMs: number;
    noProgressIterations: number;
    maxRetries: number;
  };
  const models = (subAgents.models || {}) as Record<string, AgentModelPatch>;

  const [editingId, setEditingId] = useState<BuiltinAgentId | null>(null);
  const [inheritModel, setInheritModel] = useState(true);

  const patchSubAgents = (patch: Record<string, unknown>) => {
    patchConfig({
      subAgents: normalizeWatchdogForInit({
        ...subAgents,
        ...patch,
        watchdog: {
          ...(subAgents.watchdog as Record<string, unknown>),
          ...((patch.watchdog as Record<string, unknown> | undefined) || {}),
        },
      }),
    });
  };

  const patchAgentModel = (id: BuiltinAgentId, patch: AgentModelPatch | null) => {
    const next = { ...models };
    if (patch === null) {
      delete next[id];
    } else {
      next[id] = { ...next[id], ...patch };
    }
    patchSubAgents({ models: next });
  };

  const editingAux = editingId ? (models[editingId] || {}) : {};
  const {
    models: modalModels,
    loading: loadingModels,
    error: modelsError,
    refresh: refreshModels,
  } = useProviderModels({
    providerType: editingAux.provider || p.type,
    baseUrl: editingAux.baseUrl || p.baseUrl,
    apiKey: editingAux.apiKey || p.apiKey,
    currentModel: editingAux.model,
    enabled: Boolean(editingId) && !inheritModel,
  });

  useEffect(() => {
    if (modelsError) message.warning(modelsError);
  }, [modelsError]);

  const openEditor = (id: BuiltinAgentId) => {
    setInheritModel(isAgentModelInherited(models, id));
    setEditingId(id);
  };

  const closeEditor = () => {
    setEditingId(null);
  };

  const agentModelSummary = (id: BuiltinAgentId) => {
    if (isAgentModelInherited(models, id)) {
      return `${tr('init.models.inheritTag')} · ${formatModelLine(p.type, p.model)}`;
    }
    const aux = models[id] || {};
    return formatModelLine(aux.provider || p.type, aux.model) || tr('init.models.customNoModel');
  };

  const renderModelPanel = () => {
    if (!editingId) return null;
    const aux = models[editingId] || {};
    const providerDef = meta.providers.find((item) => item.type === (aux.provider || p.type));
    const needsApiKey = providerDef?.needsApiKey !== false;

    return (
      <Form layout="vertical" className="xy-init-form-grid">
        <div className="xy-init-model-inherit-bar">
          <div>
            <strong>{tr('init.models.inheritLabel')}</strong>
            <div>
              <Text type="secondary">{tr('init.agents.inheritModelHint')}</Text>
            </div>
          </div>
          <Switch
            checked={inheritModel}
            onChange={(checked) => {
              setInheritModel(checked);
              if (checked) patchAgentModel(editingId, null);
            }}
          />
        </div>
        {!inheritModel ? (
          <>
            <Row gutter={16}>
              <Col xs={24} md={12}>
                <Form.Item label={tr('agent.provider')}>
                  <ProviderSelect
                    providers={meta.providers}
                    value={aux.provider || p.type}
                    onProviderPicked={(provider) => {
                      const def = meta.providers.find((item) => item.type === provider);
                      patchAgentModel(editingId, {
                        provider,
                        model: undefined,
                        baseUrl: def?.plans[0]?.baseUrl || aux.baseUrl,
                      });
                    }}
                  />
                </Form.Item>
              </Col>
              <Col xs={24} md={12}>
                <Form.Item label={tr('agent.baseUrl')}>
                  <Input
                    value={aux.baseUrl || ''}
                    onChange={(e) =>
                      patchAgentModel(editingId, { baseUrl: e.target.value || undefined })
                    }
                    placeholder={p.baseUrl || 'https://api.example.com/v1'}
                  />
                </Form.Item>
              </Col>
            </Row>
            {needsApiKey ? (
              <Form.Item label={tr('agent.apiKey')}>
                <Input
                  value={aux.apiKey || ''}
                  onChange={(e) => patchAgentModel(editingId, { apiKey: e.target.value || undefined })}
                  placeholder={tr('init.models.inheritApiKeyPh')}
                  autoComplete="off"
                />
              </Form.Item>
            ) : null}
            <Form.Item label={tr('agent.model')}>
              <ModelSelect
                models={modalModels}
                loading={loadingModels}
                refreshing={loadingModels}
                value={aux.model}
                onChange={(model) => patchAgentModel(editingId, { model: model || undefined })}
                placeholder={tr('init.models.inheritModelPh')}
                onRefresh={refreshModels}
                refreshLabel={tr('init.model.refreshModels')}
              />
            </Form.Item>
            <Row gutter={16}>
              <Col xs={24} md={12}>
                <Form.Item label={tr('agent.temperature')}>
                  <InputNumber
                    className="xy-init-input-full"
                    min={0}
                    max={2}
                    step={0.1}
                    value={aux.temperature}
                    placeholder={String(p.temperature ?? 0.7)}
                    onChange={(v) => patchAgentModel(editingId, { temperature: v ?? undefined })}
                  />
                </Form.Item>
              </Col>
              <Col xs={24} md={12}>
                <Form.Item label={tr('agent.maxTokens')}>
                  <InputNumber
                    className="xy-init-input-full"
                    min={32}
                    max={128000}
                    value={aux.maxTokens}
                    placeholder={String(p.maxTokens ?? 4096)}
                    onChange={(v) => patchAgentModel(editingId, { maxTokens: v ?? undefined })}
                  />
                </Form.Item>
              </Col>
            </Row>
          </>
        ) : (
          <div className="xy-init-inline-note">
            <Tag color="green">{tr('init.models.inheritTag')}</Tag>
            <span>{formatModelLine(p.type, p.model)}</span>
          </div>
        )}
      </Form>
    );
  };

  const customModelCount = useMemo(
    () => BUILTIN_AGENT_IDS.filter((id) => !isAgentModelInherited(models, id)).length,
    [models],
  );

  return (
    <>
      <Title level={4}>{tr('init.agents.title')}</Title>

      <Divider>{tr('init.agents.teamTitle')}</Divider>
      <div className="xy-init-model-grid">
        {BUILTIN_AGENT_IDS.map((id) => {
          const inherited = isAgentModelInherited(models, id);
          return (
            <Card
              key={id}
              hoverable
              className={`xy-init-model-card${!inherited ? ' is-custom' : ''}`}
              onClick={() => openEditor(id)}
            >
              <div className="xy-init-model-card-head">
                <div>
                  <strong>{subAgentRoleLabel(id === 'doc-writer' ? 'writer' : id, locale)}</strong>
                  <p>{tr(AGENT_DESC_KEYS[id])}</p>
                </div>
                <Tag color={inherited ? 'green' : 'blue'}>
                  {inherited ? tr('init.models.inheritTag') : tr('init.models.customTag')}
                </Tag>
              </div>
              <div className="xy-init-model-card-body">
                <span className="xy-init-model-card-line">{agentModelSummary(id)}</span>
              </div>
              <div className="xy-init-model-card-foot">
                <span>{tr('init.models.edit')}</span>
                <EditOutlined />
              </div>
            </Card>
          );
        })}
      </div>
      {customModelCount > 0 ? (
        <Text type="secondary" style={{ display: 'block', marginTop: 12 }}>
          {tr('init.agents.customModelCount', undefined, customModelCount)}
        </Text>
      ) : null}

      <Divider>{tr('init.agents.watchdogTitle')}</Divider>
      <Form layout="vertical">
        <Row gutter={16}>
          <Col span={8}>
            <Form.Item label={tr('init.agents.silentMs')}>
              <InputNumber
                style={{ width: '100%' }}
                min={10000}
                max={3600000}
                value={watchdog.silentMs}
                onChange={(silentMs) =>
                  patchSubAgents({
                    watchdog: { ...watchdog, silentMs: silentMs ?? 120000 },
                  })
                }
              />
            </Form.Item>
          </Col>
          <Col span={8}>
            <Form.Item label={tr('init.agents.noProgress')}>
              <InputNumber
                style={{ width: '100%' }}
                min={1}
                max={100}
                value={watchdog.noProgressIterations}
                onChange={(noProgressIterations) =>
                  patchSubAgents({
                    watchdog: {
                      ...watchdog,
                      noProgressIterations: noProgressIterations ?? 8,
                    },
                  })
                }
              />
            </Form.Item>
          </Col>
          <Col span={8}>
            <Form.Item label={tr('init.agents.maxRetries')}>
              <InputNumber
                style={{ width: '100%' }}
                min={0}
                max={10}
                value={watchdog.maxRetries}
                onChange={(maxRetries) =>
                  patchSubAgents({
                    watchdog: {
                      ...watchdog,
                      maxRetries: maxRetries ?? INIT_WATCHDOG_DEFAULTS.maxRetries,
                    },
                  })
                }
              />
            </Form.Item>
          </Col>
        </Row>
      </Form>

      <Modal
        className="xy-init-model-modal"
        rootClassName="xy-init-model-modal-root"
        open={!!editingId}
        title={
          editingId
            ? subAgentRoleLabel(editingId === 'doc-writer' ? 'writer' : editingId, locale)
            : ''
        }
        width={760}
        onCancel={closeEditor}
        footer={[
          <Button key="close" type="primary" onClick={closeEditor}>
            {tr('init.done')}
          </Button>,
        ]}
        destroyOnHidden
      >
        <div className="xy-init-segment-panel" role="tabpanel">
          {renderModelPanel()}
        </div>
      </Modal>
    </>
  );
}
