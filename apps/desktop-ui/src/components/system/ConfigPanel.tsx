/**
 * ConfigPanel — 系统设置中各配置 section 的表单内容
 * 由 SystemSettingsLayout 按 section 挂载，不再自带双层 Tabs。
 */

import React, { useEffect, useState } from 'react';
import {
  Alert,
  Button,
  Col,
  Empty,
  Form,
  Input,
  InputNumber,
  Row,
  Select,
  Space,
  Spin,
  Switch,
  Tag,
  Typography,
} from 'antd';
import { ReloadOutlined } from '@ant-design/icons';

import { useWs } from '../../App';
import { useI18n, useLocaleText } from '../../i18n';
import type { AgentDefinitionInput, ModelCapabilityRoute } from '../../protocol';
import { useProviderCatalog } from '../../hooks/useProviderCatalog';
import {
  useConfigStore,
  type AgentModelSettings,
  type AppConfig,
  type AuxiliaryModelTask,
} from '../../stores/configStore';
import { resolveVisiblePermissionMode, VISIBLE_PERMISSION_MODES } from '../../utils/permissionModes';
import { subAgentRoleLabel } from '../../utils/subagentRoles';
import { renderToolSelectOption, toolSelectOptions } from '../../utils/toolCatalog';
import {
  AGENT_THINKING_STEPS,
  EffortSlider,
  agentThinkingFromIndex,
  agentThinkingIndex,
} from '../chat/ComposerModelEffort';
import { LiveModelSelect, ProviderSelect } from '../model';
import { DataBlock, PageSurface } from '../shell/PanelShell';

const { Text } = Typography;

const AUX_MODEL_ROLES: Array<{ id: AuxiliaryModelTask; name: [string, string]; hint: [string, string] }> = [
  { id: 'title', name: ['标题生成', 'Title Generation'], hint: ['轻量、关闭深度思考', 'Lightweight, deep thinking disabled'] },
  { id: 'summary', name: ['上下文摘要', 'Context Summary'], hint: ['稳定摘要和压缩', 'Stable summarization and compression'] },
  { id: 'memory', name: ['记忆抽取', 'Memory Extraction'], hint: ['事实抽取和偏好识别', 'Fact extraction and preference detection'] },
  { id: 'intent', name: ['意图分类', 'Intent Classification'], hint: ['每轮用户消息意图推断（chat/coding/task/debug/review）', 'Per-turn user intent inference (chat/coding/task/debug/review)'] },
  { id: 'keyfacts', name: ['关键事实分类', 'Key Fact Classification'], hint: ['约束/决策/失败等行级语义分类，建议轻量 JSON 模型', 'Line-level constraint/decision/failure classification; use a lightweight JSON model'] },
];

export type ConfigSectionKey = 'models' | 'team' | 'permission' | 'context' | 'storage';

export interface ConfigSectionViewProps {
  section: ConfigSectionKey;
  config: AppConfig;
  capabilities: ModelCapabilityRoute[];
  onChange: (config: AppConfig) => void;
}

/** 被谁调用：系统设置分区。做什么：按 section 渲染配置编辑器。 */
export default function ConfigSectionView({
  section,
  config,
  capabilities,
  onChange,
}: ConfigSectionViewProps) {
  if (section === 'models') {
    return <ProviderConfig config={config} capabilities={capabilities} onChange={onChange} />;
  }
  if (section === 'team') {
    return <TeamModelConfig config={config} capabilities={capabilities} onChange={onChange} />;
  }
  if (section === 'permission') {
    return <PermissionConfig config={config} onChange={onChange} />;
  }
  if (section === 'context') {
    return <ContextConfig config={config} onChange={onChange} />;
  }
  if (section === 'storage') {
    return <StorageConfig config={config} onChange={onChange} />;
  }
  return null;
}

interface EditorProps {
  config: AppConfig;
  capabilities: ModelCapabilityRoute[];
  onChange: (config: AppConfig) => void;
}

interface BasicEditorProps {
  config: AppConfig;
  onChange: (config: AppConfig) => void;
}

/** 被谁调用：ConfigPanel。做什么：主模型 provider 表单。 */
function ProviderConfig({ config, capabilities, onChange }: EditorProps) {
  const lt = useLocaleText();
  const providers = useProviderCatalog();
  const mainCapability = capabilities.find((item) => item.task === 'main');

  return (
    <PageSurface>
      <DataBlock title={lt('主模型', 'Main Model')}>
        <Form layout="vertical" className="xy-settings-form">
          <Row gutter={[20, 0]}>
            <Col xs={24} md={12}>
              <Form.Item label={lt('供应商', 'Provider')}>
                <ProviderSelect
                  providers={providers}
                  value={config.provider.type}
                  onProviderPicked={(type, entry) =>
                    onChange({
                      ...config,
                      provider: {
                        ...config.provider,
                        type,
                        model: '',
                        baseUrl: entry?.defaultBaseUrl || config.provider.baseUrl,
                      },
                    })
                  }
                />
              </Form.Item>
              <Form.Item label={lt('模型', 'Model')}>
                <LiveModelSelect
                  providerType={config.provider.type}
                  baseUrl={config.provider.baseUrl}
                  apiKey={config.provider.apiKey}
                  allowClear={false}
                  value={config.provider.model}
                  refreshLabel={lt('刷新', 'Refresh')}
                  onChange={(model) =>
                    onChange({
                      ...config,
                      provider: { ...config.provider, model: model || config.provider.model },
                    })
                  }
                />
              </Form.Item>
              <Form.Item label={lt('接口密钥', 'API Key')}>
                <Input
                  value={config.provider.apiKey}
                  onChange={(e) =>
                    onChange({ ...config, provider: { ...config.provider, apiKey: e.target.value } })
                  }
                  placeholder="sk-..."
                  autoComplete="off"
                />
              </Form.Item>
              <Form.Item label={lt('基础地址', 'Base URL')}>
                <Input
                  value={config.provider.baseUrl}
                  onChange={(e) =>
                    onChange({ ...config, provider: { ...config.provider, baseUrl: e.target.value } })
                  }
                />
              </Form.Item>
            </Col>
            <Col xs={24} md={12}>
              <Form.Item label={lt('最大输出', 'Max Tokens')}>
                <InputNumber
                  style={{ width: '100%' }}
                  min={1}
                  value={config.provider.maxTokens}
                  onChange={(value) =>
                    onChange({
                      ...config,
                      provider: { ...config.provider, maxTokens: Number(value || 0) },
                    })
                  }
                />
              </Form.Item>
              <Form.Item label={lt('温度', 'Temperature')}>
                <InputNumber
                  style={{ width: '100%' }}
                  min={0}
                  max={2}
                  step={0.1}
                  value={config.provider.temperature}
                  onChange={(value) =>
                    onChange({
                      ...config,
                      provider: { ...config.provider, temperature: Number(value || 0) },
                    })
                  }
                />
              </Form.Item>
              <Form.Item label={lt('最大上下文', 'Max Context')}>
                <InputNumber
                  style={{ width: '100%' }}
                  min={1024}
                  value={config.maxContextTokens}
                  onChange={(value) =>
                    onChange({ ...config, maxContextTokens: Number(value || 0) })
                  }
                />
              </Form.Item>
            </Col>
          </Row>
          {mainCapability && <CapabilityStrip route={mainCapability} />}
        </Form>
      </DataBlock>
    </PageSurface>
  );
}

/** 被谁调用：ConfigPanel。做什么：团队/辅助模型路由表单。 */
function TeamModelConfig({ config, capabilities, onChange }: EditorProps) {
  const { send, addHandler } = useWs();
  const { locale } = useI18n();
  const lt = useLocaleText();
  const providers = useProviderCatalog();
  const [agents, setAgents] = useState<AgentDefinitionInput[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    send('agent.list', {});
    return addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type === 'agent.list') {
        const payload = msg.payload as { agents: AgentDefinitionInput[] };
        setAgents(payload.agents || []);
        setLoading(false);
      }
    });
  }, [addHandler, send]);

  const subAgents = config.subAgents || {
    configDir: 'capabilities/agents',
    models: {},
  };
  const models = subAgents.models || {};
  const modelRouting = config.modelRouting || {
    enabled: true,
    auxiliaryModels: {},
    rateLimits: {},
    capabilities: {},
  };
  const auxiliaryModels = modelRouting.auxiliaryModels || {};
  const mainProvider = config.provider;
  const mainModelLine = `${mainProvider.type}/${mainProvider.model}`;

  const writeRoleModels = (nextModels: Record<string, AgentModelSettings>) => {
    onChange({
      ...config,
      subAgents: {
        ...subAgents,
        models: nextModels,
      },
    });
  };

  const updateRole = (role: string, patch: Record<string, unknown>) => {
    writeRoleModels({
      ...models,
      [role]: {
        ...(models[role] || {}),
        ...patch,
      },
    });
  };

  const setRoleInherited = (role: string, inherited: boolean) => {
    if (inherited) {
      const next = { ...models };
      delete next[role];
      writeRoleModels(next);
      return;
    }
    updateRole(role, {
      provider: mainProvider.type,
      model: mainProvider.model,
      baseUrl: mainProvider.baseUrl,
      apiKey: mainProvider.apiKey,
      maxTokens: mainProvider.maxTokens,
      temperature: mainProvider.temperature,
    });
  };

  const writeAuxModels = (nextAux: Partial<Record<AuxiliaryModelTask, AgentModelSettings>>) => {
    onChange({
      ...config,
      modelRouting: {
        ...modelRouting,
        auxiliaryModels: nextAux,
      },
    });
  };

  const updateAux = (role: AuxiliaryModelTask, patch: Record<string, unknown>) => {
    writeAuxModels({
      ...auxiliaryModels,
      [role]: {
        ...(auxiliaryModels[role] || {}),
        ...patch,
      },
    });
  };

  const setAuxInherited = (role: AuxiliaryModelTask, inherited: boolean) => {
    if (inherited) {
      const next = { ...auxiliaryModels };
      delete next[role];
      writeAuxModels(next);
      return;
    }
    updateAux(role, {
      provider: mainProvider.type,
      model: mainProvider.model,
      baseUrl: mainProvider.baseUrl,
      apiKey: mainProvider.apiKey,
      maxTokens: mainProvider.maxTokens,
      temperature: mainProvider.temperature,
    });
  };

  const inheritAll = () => {
    onChange({
      ...config,
      subAgents: {
        ...subAgents,
        models: {},
      },
      modelRouting: {
        ...modelRouting,
        auxiliaryModels: {},
      },
    });
  };

  return (
    <div className="xy-settings-stack">
      <PageSurface>
        <DataBlock
          title={lt('Agent 团队模型', 'Agent Team Models')}
          extra={
            <Space size={6}>
              <Button size="small" onClick={inheritAll}>
                {lt('全部使用主模型', 'Use main model for all')}
              </Button>
              <Button icon={<ReloadOutlined />} size="small" onClick={() => send('agent.list', {})}>
                {lt('刷新列表', 'Refresh list')}
              </Button>
            </Space>
          }
        >
          <Text type="secondary" className="xy-settings-hint">
            {lt(
              '可跟随主模型，或单独配置供应商、密钥与模型。覆盖优先于 Agent 自身模型设置。',
              'Inherit the main model, or set provider, API key, and model per agent. Overrides take precedence over each Agent model setting.',
            )}
          </Text>
          {loading ? (
            <Spin tip={lt('加载 Agent 列表...', 'Loading Agent list...')} />
          ) : agents.length === 0 ? (
            <Empty
              description={lt(
                '暂无 Agent，请先在「子 Agent」页创建',
                'No Agents yet. Create one in the Sub-Agents page first.',
              )}
            />
          ) : (
            <div className="xy-settings-entity-list">
              {agents.map((agent) => {
                const value = models[agent.id] || {};
                const inherited = !value.model && !value.provider;
                return (
                  <div key={agent.id} className="xy-settings-entity">
                    <div className="xy-settings-entity-head">
                      <div>
                        <strong>
                          {agent.name}{' '}
                          <Text type="secondary" code>
                            {agent.id}
                          </Text>
                        </strong>
                        {agent.description ? (
                          <div className="xy-settings-entity-meta">{agent.description}</div>
                        ) : null}
                      </div>
                      <Space size={6}>
                        {agent.role && <Tag>{subAgentRoleLabel(agent.role, locale)}</Tag>}
                        <Tag color={inherited ? 'default' : 'processing'}>
                          {inherited ? lt('跟随主模型', 'Inherited') : lt('单独配置', 'Custom')}
                        </Tag>
                      </Space>
                    </div>
                    <div className="xy-agent-inherit-bar">
                      <div>
                        <strong>{lt('跟随主模型', 'Use main model')}</strong>
                        <div className="xy-agent-inherit-hint">{mainModelLine}</div>
                      </div>
                      <Switch
                        checked={inherited}
                        onChange={(checked) => setRoleInherited(agent.id, checked)}
                      />
                    </div>
                    {inherited ? (
                      <Tag color="green">{lt('跟随主模型', 'Inherited')}</Tag>
                    ) : (
                      <Form layout="vertical" className="xy-settings-form">
                        <Row gutter={12}>
                          <Col xs={24} md={12}>
                            <Form.Item label={lt('供应商', 'Provider')}>
                              <ProviderSelect
                                providers={providers}
                                value={value.provider || mainProvider.type}
                                onProviderPicked={(provider, entry) =>
                                  updateRole(agent.id, {
                                    provider,
                                    model: undefined,
                                    baseUrl: entry?.defaultBaseUrl || value.baseUrl,
                                  })
                                }
                              />
                            </Form.Item>
                          </Col>
                          <Col xs={24} md={12}>
                            <Form.Item label={lt('模型', 'Model')}>
                              <LiveModelSelect
                                providerType={value.provider || mainProvider.type}
                                baseUrl={value.baseUrl || mainProvider.baseUrl}
                                apiKey={value.apiKey || mainProvider.apiKey}
                                value={value.model || undefined}
                                placeholder={mainProvider.model}
                                showRefresh
                                onChange={(model) =>
                                  updateRole(agent.id, { model: model || undefined })
                                }
                              />
                            </Form.Item>
                          </Col>
                          <Col xs={24} md={12}>
                            <Form.Item label={lt('接口密钥', 'API Key')}>
                              <Input
                                value={value.apiKey || ''}
                                placeholder={lt('留空则用主模型密钥', 'Empty = main API key')}
                                autoComplete="off"
                                onChange={(e) =>
                                  updateRole(agent.id, { apiKey: e.target.value || undefined })
                                }
                              />
                            </Form.Item>
                          </Col>
                          <Col xs={24} md={12}>
                            <Form.Item label={lt('基础地址', 'Base URL')}>
                              <Input
                                value={value.baseUrl || ''}
                                placeholder={mainProvider.baseUrl || 'https://...'}
                                onChange={(e) =>
                                  updateRole(agent.id, { baseUrl: e.target.value || undefined })
                                }
                              />
                            </Form.Item>
                          </Col>
                          <Col xs={24} md={8}>
                            <Form.Item label={lt('最大输出', 'Max Tokens')}>
                              <InputNumber
                                style={{ width: '100%' }}
                                min={256}
                                max={128000}
                                value={value.maxTokens}
                                placeholder={String(mainProvider.maxTokens ?? 4096)}
                                onChange={(v) =>
                                  updateRole(agent.id, {
                                    maxTokens: typeof v === 'number' ? v : undefined,
                                  })
                                }
                              />
                            </Form.Item>
                          </Col>
                          <Col xs={24} md={8}>
                            <Form.Item label={lt('温度', 'Temperature')}>
                              <InputNumber
                                style={{ width: '100%' }}
                                min={0}
                                max={2}
                                step={0.1}
                                value={value.temperature}
                                placeholder={String(mainProvider.temperature ?? 0.7)}
                                onChange={(v) =>
                                  updateRole(agent.id, {
                                    temperature: typeof v === 'number' ? v : undefined,
                                  })
                                }
                              />
                            </Form.Item>
                          </Col>
                          <Col xs={24} md={8}>
                            <Form.Item label={lt('深度思考', 'Deep thinking')}>
                              {(() => {
                                const deepThinking = value.deepThinking === true;
                                const effort = (value.reasoningEffort || 'high') as
                                  | 'low'
                                  | 'medium'
                                  | 'high'
                                  | 'max'
                                  | 'ultracode';
                                const index = agentThinkingIndex(deepThinking, effort);
                                const step = AGENT_THINKING_STEPS[index] || 'off';
                                const label =
                                  step === 'off'
                                    ? lt('关', 'Off')
                                    : lt(
                                        (
                                          {
                                            low: '低',
                                            medium: '中',
                                            high: '高',
                                            max: '极高',
                                            ultracode: '最强',
                                          } as Record<string, string>
                                        )[step] || '高',
                                        step,
                                      );
                                return (
                                  <div
                                    className={`xy-agent-thinking${
                                      deepThinking && effort === 'ultracode' ? ' is-ultracode' : ''
                                    }`}
                                  >
                                    <span className="xy-agent-thinking-label">{label}</span>
                                    <EffortSlider
                                      value={index}
                                      max={AGENT_THINKING_STEPS.length - 1}
                                      isUltracode={deepThinking && effort === 'ultracode'}
                                      onChange={(next) =>
                                        updateRole(agent.id, agentThinkingFromIndex(next))
                                      }
                                    />
                                  </div>
                                );
                              })()}
                            </Form.Item>
                          </Col>
                        </Row>
                      </Form>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </DataBlock>
      </PageSurface>

      <PageSurface>
        <DataBlock title={lt('辅助任务模型路由', 'Auxiliary Model Routing')}>
          <Text type="secondary" className="xy-settings-hint">
            {lt(
              '标题、摘要、记忆等可跟随主模型，或单独配置供应商、密钥与模型。',
              'Title, summary, memory, and related tasks can inherit the main model or use a custom provider, API key, and model.',
            )}
          </Text>
          <div className="xy-settings-entity-list">
            {AUX_MODEL_ROLES.map((role) => {
              const value = auxiliaryModels[role.id] || {};
              const inherited = !value.model && !value.provider;
              return (
                <div key={role.id} className="xy-settings-entity">
                  <div className="xy-settings-entity-head">
                    <div>
                      <strong>{lt(...role.name)}</strong>
                      <div className="xy-settings-entity-meta">{lt(...role.hint)}</div>
                    </div>
                    <Tag color={inherited ? 'default' : 'processing'}>
                      {inherited ? lt('跟随主模型', 'Inherited') : lt('单独配置', 'Custom')}
                    </Tag>
                  </div>
                  <div className="xy-agent-inherit-bar">
                    <div>
                      <strong>{lt('跟随主模型', 'Use main model')}</strong>
                      <div className="xy-agent-inherit-hint">{mainModelLine}</div>
                    </div>
                    <Switch
                      checked={inherited}
                      onChange={(checked) => setAuxInherited(role.id, checked)}
                    />
                  </div>
                  {inherited ? (
                    <Tag color="green">{lt('跟随主模型', 'Inherited')}</Tag>
                  ) : (
                    <Form layout="vertical" className="xy-settings-form">
                      <Row gutter={12}>
                        <Col xs={24} md={12}>
                          <Form.Item label={lt('供应商', 'Provider')}>
                            <ProviderSelect
                              providers={providers}
                              value={value.provider || mainProvider.type}
                              onProviderPicked={(provider, entry) =>
                                updateAux(role.id, {
                                  provider,
                                  model: undefined,
                                  baseUrl: entry?.defaultBaseUrl || value.baseUrl,
                                })
                              }
                            />
                          </Form.Item>
                        </Col>
                        <Col xs={24} md={12}>
                          <Form.Item label={lt('模型', 'Model')}>
                            <LiveModelSelect
                              providerType={value.provider || mainProvider.type}
                              baseUrl={value.baseUrl || mainProvider.baseUrl}
                              apiKey={value.apiKey || mainProvider.apiKey}
                              value={value.model || undefined}
                              placeholder={mainProvider.model}
                              showRefresh
                              onChange={(model) =>
                                updateAux(role.id, { model: model || undefined })
                              }
                            />
                          </Form.Item>
                        </Col>
                        <Col xs={24} md={12}>
                          <Form.Item label={lt('接口密钥', 'API Key')}>
                            <Input
                              value={value.apiKey || ''}
                              placeholder={lt('留空则用主模型密钥', 'Empty = main API key')}
                              autoComplete="off"
                              onChange={(e) =>
                                updateAux(role.id, { apiKey: e.target.value || undefined })
                              }
                            />
                          </Form.Item>
                        </Col>
                        <Col xs={24} md={12}>
                          <Form.Item label={lt('基础地址', 'Base URL')}>
                            <Input
                              value={value.baseUrl || ''}
                              placeholder={mainProvider.baseUrl || 'https://...'}
                              onChange={(e) =>
                                updateAux(role.id, { baseUrl: e.target.value || undefined })
                              }
                            />
                          </Form.Item>
                        </Col>
                        <Col xs={24} md={12}>
                          <Form.Item label={lt('最大输出', 'Max Tokens')}>
                            <InputNumber
                              style={{ width: '100%' }}
                              min={256}
                              max={128000}
                              value={value.maxTokens}
                              placeholder={String(mainProvider.maxTokens ?? 4096)}
                              onChange={(v) =>
                                updateAux(role.id, {
                                  maxTokens: typeof v === 'number' ? v : undefined,
                                })
                              }
                            />
                          </Form.Item>
                        </Col>
                        <Col xs={24} md={12}>
                          <Form.Item label={lt('温度', 'Temperature')}>
                            <InputNumber
                              style={{ width: '100%' }}
                              min={0}
                              max={2}
                              step={0.1}
                              value={value.temperature}
                              placeholder={String(mainProvider.temperature ?? 0.7)}
                              onChange={(v) =>
                                updateAux(role.id, {
                                  temperature: typeof v === 'number' ? v : undefined,
                                })
                              }
                            />
                          </Form.Item>
                        </Col>
                      </Row>
                    </Form>
                  )}
                </div>
              );
            })}
          </div>
        </DataBlock>
      </PageSurface>
    </div>
  );
}

/** 被谁调用：ConfigPanel。做什么：记忆与存储后端表单。 */
function StorageConfig({ config, onChange }: BasicEditorProps) {
  const lt = useLocaleText();
  const memory = config.memory || {
    backend: 'postgres' as const,
    maxEntries: 1000,
    cleanupImportanceThreshold: 3,
  };
  return (
    <PageSurface>
      <DataBlock title={lt('PostgreSQL', 'PostgreSQL')}>
        <Form layout="vertical" className="xy-settings-form">
          <Form.Item
            label={lt('连接串', 'Connection string')}
            extra={lt(
              '会话与长期记忆共用此 PostgreSQL（含 pgvector）。改连接后需重启本地引擎。',
              'Sessions and long-term memory share this PostgreSQL (with pgvector). Restart the engine after changing the connection.',
            )}
          >
            <Input
              value={memory.postgres?.connectionString}
              placeholder="postgres://user@127.0.0.1:55432/xuanyu"
              onChange={(e) =>
                onChange({
                  ...config,
                  memory: {
                    ...memory,
                    backend: 'postgres',
                    postgres: { ...memory.postgres, connectionString: e.target.value },
                  },
                })
              }
            />
          </Form.Item>
        </Form>
      </DataBlock>
    </PageSurface>
  );
}

/** 被谁调用：ConfigPanel。做什么：权限模式与规则表单。 */
function PermissionConfig({ config, onChange }: BasicEditorProps) {
  const { send } = useWs();
  const { locale } = useI18n();
  const lt = useLocaleText();
  const toolCatalog = useConfigStore((s) => s.toolCatalog);
  const permissionModeOptions = VISIBLE_PERMISSION_MODES.map((mode) => ({
    value: mode,
    label:
      mode === 'default'
        ? lt('默认', 'Default')
        : mode === 'plan'
          ? lt('计划', 'Plan')
          : lt('自动', 'Auto'),
  }));

  useEffect(() => {
    send('tools.catalog.get', {});
  }, [send]);

  return (
    <div className="xy-settings-stack">
      <PageSurface>
        <DataBlock title={lt('权限', 'Permissions')}>
          <Form layout="vertical" className="xy-settings-form">
            <Form.Item
              label={lt('权限运行模式', 'Permission mode')}
              extra={lt(
                '默认：敏感操作确认；计划：只读规划；自动：尽量少打断。',
                'Default: confirm sensitive tools; Plan: read-only planning; Auto: fewer prompts.',
              )}
            >
              <Select
                value={resolveVisiblePermissionMode(config.permissions.mode)}
                onChange={(mode) =>
                  onChange({ ...config, permissions: { ...config.permissions, mode } })
                }
                options={permissionModeOptions}
              />
            </Form.Item>
            <Form.Item label={lt('禁用工具', 'Disabled Tools')}>
              <Select
                mode="tags"
                value={config.disabledTools}
                options={toolSelectOptions(toolCatalog, locale)}
                optionRender={(option) => renderToolSelectOption(option, locale)}
                optionFilterProp="title"
                onChange={(disabledTools) => onChange({ ...config, disabledTools })}
                placeholder={lt('输入工具名后回车', 'Type a tool name and press Enter')}
              />
            </Form.Item>
          </Form>
        </DataBlock>
      </PageSurface>

      <PageSurface>
        <DataBlock title={lt('自动修复', 'Auto Fix')}>
          <Form layout="vertical" className="xy-settings-form">
            <Form.Item
              label={lt('验证失败后自动修复轮次', 'Auto-fix passes after validation failure')}
              extra={lt(
                '主 Agent turn 结束验证失败时，最多再开几轮自动改代码（默认 5）',
                'After turn-end validation fails, max automatic fix rounds (default 5)',
              )}
            >
              <InputNumber
                min={1}
                max={8}
                value={config.agentLoop?.maxAutoFixPasses ?? 5}
                onChange={(value) => {
                  const maxAutoFixPasses = typeof value === 'number' ? value : 5;
                  onChange({
                    ...config,
                    agentLoop: {
                      ...(config.agentLoop || {}),
                      maxAutoFixPasses,
                    },
                  });
                }}
              />
            </Form.Item>
          </Form>
        </DataBlock>
      </PageSurface>
    </div>
  );
}

/** 被谁调用：ConfigPanel。做什么：上下文压缩与预算表单。 */
function ContextConfig({ config, onChange }: BasicEditorProps) {
  const lt = useLocaleText();
  const memory = config.memory || {
    backend: 'postgres' as const,
    maxEntries: 1000,
    cleanupImportanceThreshold: 3,
  };
  const contextManagement = config.contextManagement || {};
  return (
    <div className="xy-settings-stack">
      <PageSurface>
        <DataBlock title={lt('上下文', 'Context')}>
          <Form layout="vertical" className="xy-settings-form">
            <Row gutter={16}>
              <Col xs={24} md={12}>
                <Form.Item
                  label={lt('自动压缩阈值', 'Auto-compact threshold')}
                  extra={lt(
                    '输入上下文占用超过该比例时触发自动压缩（默认 0.85）。',
                    'Trigger auto-compaction when input context usage exceeds this ratio (default 0.85).',
                  )}
                >
                  <InputNumber
                    style={{ width: '100%' }}
                    min={0.5}
                    max={0.99}
                    step={0.01}
                    value={contextManagement.autoCompactThreshold}
                    onChange={(value) =>
                      onChange({
                        ...config,
                        contextManagement: {
                          ...contextManagement,
                          autoCompactThreshold: value === null ? undefined : Number(value),
                        },
                      })
                    }
                  />
                </Form.Item>
              </Col>
              <Col xs={24} md={12}>
                <Form.Item
                  label={lt('记忆自动注入阈值', 'Memory auto-inject min score')}
                  extra={lt(
                    '相关记忆达到该分数才会自动注入本轮上下文。',
                    'Only auto-inject memories that score at or above this threshold.',
                  )}
                >
                  <InputNumber
                    style={{ width: '100%' }}
                    min={0}
                    max={1}
                    step={0.05}
                    value={contextManagement.memoryAutoInjectMinScore}
                    onChange={(value) =>
                      onChange({
                        ...config,
                        contextManagement: {
                          ...contextManagement,
                          memoryAutoInjectMinScore: value === null ? undefined : Number(value),
                        },
                      })
                    }
                  />
                </Form.Item>
              </Col>
            </Row>
          </Form>
        </DataBlock>
      </PageSurface>

      <PageSurface>
        <DataBlock title={lt('记忆保留', 'Memory retention')}>
          <Form layout="vertical" className="xy-settings-form">
            <Row gutter={16}>
              <Col xs={24} md={12}>
                <Form.Item label={lt('最大记忆条数', 'Max memories')}>
                  <InputNumber
                    style={{ width: '100%' }}
                    min={1}
                    value={memory.maxEntries}
                    onChange={(value) =>
                      onChange({
                        ...config,
                        memory: { ...memory, maxEntries: Number(value || 1) },
                      })
                    }
                  />
                </Form.Item>
              </Col>
              <Col xs={24} md={12}>
                <Form.Item
                  label={lt('清理重要度阈值', 'Cleanup importance threshold')}
                  extra={lt(
                    '低于该重要度的旧记忆更易被清理（0–10）。',
                    'Older memories below this importance are cleaned first (0–10).',
                  )}
                >
                  <InputNumber
                    style={{ width: '100%' }}
                    min={0}
                    max={10}
                    value={memory.cleanupImportanceThreshold}
                    onChange={(value) =>
                      onChange({
                        ...config,
                        memory: { ...memory, cleanupImportanceThreshold: Number(value || 0) },
                      })
                    }
                  />
                </Form.Item>
              </Col>
            </Row>
          </Form>
        </DataBlock>
      </PageSurface>
    </div>
  );
}

/** 被谁调用：模型配置。做什么：展示路由能力条。 */
function CapabilityStrip({ route }: { route: ModelCapabilityRoute }) {
  const lt = useLocaleText();
  const caps = route.capabilities;
  const issues = [...route.check.errors, ...route.check.warnings];
  return (
    <div className="xy-settings-capability">
      <Space wrap size={[6, 6]}>
        <Tag color={route.check.errors.length ? 'error' : route.check.warnings.length ? 'warning' : 'default'}>
          {route.provider}/{route.model}
        </Tag>
        <CapabilityTag label={lt('流式', 'stream')} ok={caps.stream} />
        <CapabilityTag label={lt('工具', 'tool')} ok={caps.toolCall} />
        <CapabilityTag label={lt('推理', 'reasoning')} ok={caps.reasoning} neutral />
        <CapabilityTag label={lt('视觉', 'vision')} ok={caps.vision} />
        {caps.maxContext && (
          <Tag>
            {lt('上下文', 'ctx')} {formatNumber(caps.maxContext)}
          </Tag>
        )}
      </Space>
      {issues.length > 0 && (
        <Alert
          style={{ marginTop: 8 }}
          type={route.check.errors.length ? 'error' : 'warning'}
          showIcon
          message={issues.slice(0, 3).join('；')}
        />
      )}
    </div>
  );
}

/** 被谁调用：CapabilityStrip。做什么：单项能力 OK/缺失标签。 */
function CapabilityTag({ label, ok, neutral }: { label: string; ok: boolean; neutral?: boolean }) {
  const lt = useLocaleText();
  return (
    <Tag color={neutral ? (ok ? 'processing' : 'default') : ok ? 'success' : 'default'}>
      {label}: {ok ? lt('是', 'yes') : lt('否', 'no')}
    </Tag>
  );
}

/** ≥1e6 → n.nM；≥1000 → nK。 */
function formatNumber(value: number): string {
  if (value >= 1000000) return `${Math.round(value / 100000) / 10}M`;
  if (value >= 1000) return `${Math.round(value / 1000)}K`;
  return String(value);
}
