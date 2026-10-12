/**
 * AgentPanel — 子 Agent 团队管理（纯配置）
 *
 * 名册增删改；内置可编辑覆盖、不可删除。模型选型：供应商/模型下拉（与 init 向导同一套）。
 * 能否写文件由工具集决定，不另设开关。
 */

import React, { useEffect, useState, useCallback } from 'react';
import {
  Button, Drawer, Empty, Form, Input, InputNumber, Popconfirm, Select, Space,
  Switch, Table, Tag, message,
} from 'antd';
import {
  DeleteOutlined, EditOutlined, PlusOutlined,
} from '@ant-design/icons';

import { useWs } from '../../App';
import { useI18n } from '../../i18n';
import type {
  AgentDefinitionInput,
  SubAgentTeamMember,
  SubAgentTeamSnapshot,
} from '../../protocol';
import { registerTeamSnapshotSetter } from '../../hooks/useWebSocket';
import { useProviderCatalog } from '../../hooks/useProviderCatalog';
import { useChatStore } from '../../stores/chatStore';
import { useConfigStore, type AgentModelSettings, type AppConfig } from '../../stores/configStore';
import { useSessionStore } from '../../stores/sessionStore';
import { resolveVisiblePermissionMode, VISIBLE_PERMISSION_MODES } from '../../utils/permissionModes';
import { subAgentRoleLabel } from '../../utils/subagentRoles';
import { renderToolSelectOption, toolSelectOptions } from '../../utils/toolCatalog';
import {
  AGENT_THINKING_STEPS,
  EffortSlider,
  agentThinkingFromIndex,
  agentThinkingIndex,
  type ReasoningEffort,
} from '../chat/ComposerModelEffort';
import { LiveModelSelect, ProviderSelect } from '../model';

const { TextArea } = Input;

const PERMISSION_MODES = VISIBLE_PERMISSION_MODES;
const SANDBOX_MODES = ['workspace', 'worktree', 'readonly'] as const;
const MEMORY_SCOPES = ['none', 'session', 'project'] as const;
const ROLE_OPTIONS = ['architect', 'developer', 'explorer', 'reviewer', 'doc-writer'] as const;

const OPTION_LABELS: Record<string, [string, string]> = {
  default: ['默认', 'Default'],
  plan: ['计划', 'Plan'],
  auto: ['自动', 'Auto'],
  worktree: ['独立工作树', 'Worktree'],
  workspace: ['工作区', 'Workspace'],
  readonly: ['只读', 'Read-only'],
  none: ['无', 'None'],
  session: ['会话', 'Session'],
  project: ['项目', 'Project'],
};

/** 被谁调用：AgentPanel。做什么：判断是否沿用主模型。 */
function isInheritedModel(agent: Partial<AgentDefinitionInput> | null | undefined): boolean {
  if (!agent) return true;
  return !agent.provider && !agent.model && !agent.apiKey && !agent.baseUrl;
}

/** 被谁调用：任务导航。做什么：Agent 团队 CRUD/启用与模型覆盖。 */
export default function AgentPanel() {
  const { send, addHandler } = useWs();
  const { t, locale } = useI18n();
  const config = useConfigStore((s) => s.config);
  const updateConfig = useConfigStore((s) => s.updateConfig);
  const toolCatalog = useConfigStore((s) => s.toolCatalog);
  const sessions = useSessionStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const activeSession = sessions.find((session) => session.id === activeSessionId) ||
    sessions.find((session) => session.isProcessing) ||
    sessions[0];
  const [team, setTeam] = useState<SubAgentTeamSnapshot | null>(null);
  const [agents, setAgents] = useState<AgentDefinitionInput[]>([]);
  const providers = useProviderCatalog();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [editingAgent, setEditingAgent] = useState<AgentDefinitionInput | null>(null);
  const [inheritModel, setInheritModel] = useState(true);
  const [form] = Form.useForm();
  const toolOptions = toolSelectOptions(toolCatalog, locale);
  const watchedProvider = Form.useWatch('provider', form) as string | undefined;
  const watchedBaseUrl = Form.useWatch('baseUrl', form) as string | undefined;
  const watchedApiKey = Form.useWatch('apiKey', form) as string | undefined;
  const watchedModel = Form.useWatch('model', form) as string | undefined;

  useEffect(() => {
    registerTeamSnapshotSetter(setTeam);
    return () => registerTeamSnapshotSetter(null);
  }, []);

  useEffect(() => {
    send('extensions.list', {});
    send('team.status', { sessionId: activeSession?.id });
    send('agent.list', {});
    send('tools.catalog.get', { sessionId: activeSession?.id });
  }, [send, activeSession?.id]);

  useEffect(() => {
    return addHandler((msg: { type: string; payload: unknown }) => {
      if (msg.type === 'agent.list') {
        const payload = msg.payload as { agents: AgentDefinitionInput[] };
        setAgents(payload.agents || []);
      }
      if (msg.type === 'agent.created' || msg.type === 'agent.updated' || msg.type === 'agent.deleted') {
        send('agent.list', {});
        send('team.status', { sessionId: activeSession?.id });
      }
      if (msg.type === 'config.updated') {
        send('team.status', { sessionId: activeSession?.id });
      }
    });
  }, [addHandler, send, activeSession?.id]);

  const patchAgentModel = useCallback(
    (agentId: string, patch: Partial<AgentModelSettings>) => {
      if (!config) return;
      const subAgents = config.subAgents || {
        models: {},
      };
      const prev = subAgents.models?.[agentId] || {};
      const nextModels = {
        ...(subAgents.models || {}),
        [agentId]: {
          ...prev,
          ...patch,
        },
      };
      const nextSubAgents: NonNullable<AppConfig['subAgents']> = {
        ...subAgents,
        models: nextModels,
      };
      const nextPartial = { subAgents: nextSubAgents };
      updateConfig(nextPartial);
      send('config.update', { config: nextPartial });
    },
    [config, send, updateConfig],
  );

  const handleCreate = useCallback(() => {
    setEditingAgent(null);
    setInheritModel(true);
    form.resetFields();
    form.setFieldsValue({
      role: 'developer',
      permissionMode: 'default',
      memoryScope: 'session',
      defaultSandboxMode: 'workspace',
      tools: ['file_read', 'file_write', 'file_edit', 'search', 'glob', 'list_files'],
      enabled: true,
      provider: undefined,
      model: undefined,
      baseUrl: undefined,
      apiKey: undefined,
      maxTokens: undefined,
      temperature: undefined,
    });
    setDrawerOpen(true);
  }, [form]);

  const handleEdit = useCallback((agent: AgentDefinitionInput) => {
    setEditingAgent(agent);
    const inherited = isInheritedModel(agent);
    setInheritModel(inherited);
    form.setFieldsValue({
      id: agent.id,
      name: agent.name,
      role: agent.role,
      description: agent.description,
      prompt: agent.prompt,
      permissionMode: resolveVisiblePermissionMode(agent.permissionMode),
      defaultSandboxMode: agent.defaultSandboxMode,
      memoryScope: agent.memoryScope,
      tools: agent.tools || [],
      provider: inherited ? undefined : agent.provider || config?.provider?.type,
      model: inherited ? undefined : agent.model,
      baseUrl: inherited ? undefined : agent.baseUrl,
      apiKey: inherited ? undefined : agent.apiKey,
      maxTokens: inherited ? undefined : agent.maxTokens,
      temperature: inherited ? undefined : agent.temperature,
    });
    setDrawerOpen(true);
  }, [form, config?.provider?.type]);

  const handleDelete = useCallback((id: string) => {
    const agent = agents.find((item) => item.id === id);
    if (agent?.source === 'builtin') {
      message.warning(t('agent.builtinDeleteBlocked'));
      return;
    }
    send('agent.delete', { id });
    message.success(t('common.deleted'));
  }, [agents, send, t]);

  const handleEnabledChange = useCallback((agent: AgentDefinitionInput, enabled: boolean) => {
    setAgents((prev) => prev.map((item) => (item.id === agent.id ? { ...item, enabled } : item)));
    send('agent.update', {
      id: agent.id,
      name: agent.name,
      description: agent.description,
      prompt: agent.prompt,
      tools: agent.tools,
      permissionMode: agent.permissionMode,
      provider: agent.provider,
      model: agent.model,
      baseUrl: agent.baseUrl,
      apiKey: agent.apiKey,
      maxTokens: agent.maxTokens,
      temperature: agent.temperature,
      defaultSandboxMode: agent.defaultSandboxMode,
      memoryScope: agent.memoryScope,
      role: agent.role,
      enabled,
    });
  }, [send]);

  const handleInheritChange = useCallback((checked: boolean) => {
    setInheritModel(checked);
    if (checked) {
      form.setFieldsValue({
        provider: undefined,
        model: undefined,
        baseUrl: undefined,
        apiKey: undefined,
        maxTokens: undefined,
        temperature: undefined,
      });
      return;
    }
    const main = config?.provider;
    form.setFieldsValue({
      provider: main?.type,
      model: main?.model,
      baseUrl: main?.baseUrl,
      apiKey: undefined,
      maxTokens: main?.maxTokens,
      temperature: main?.temperature,
    });
  }, [config?.provider, form]);

  const handleProviderChange = useCallback((provider: string, entry?: { defaultBaseUrl?: string }) => {
    form.setFieldsValue({
      provider,
      model: undefined,
      baseUrl: entry?.defaultBaseUrl,
    });
  }, [form]);

  const handleSave = useCallback(() => {
    form.validateFields().then((values) => {
      const payload = {
        ...values,
        enabled: editingAgent ? editingAgent.enabled !== false : values.enabled !== false,
        ...(inheritModel
          ? {
              provider: undefined,
              model: undefined,
              baseUrl: undefined,
              apiKey: undefined,
              maxTokens: undefined,
              temperature: undefined,
            }
          : {}),
      };
      if (editingAgent) {
        send('agent.update', { id: editingAgent.id, ...payload });
        message.success(t('common.updated'));
      } else {
        send('agent.create', payload);
        message.success(t('common.created'));
      }
      setDrawerOpen(false);
    });
  }, [form, editingAgent, inheritModel, send, t]);

  const members = team?.members || [];
  const memberByAgentId = new Map(members.map((m) => [m.agentId, m]));
  const modelOverrides = config?.subAgents?.models || {};
  const mainModelLine = config?.provider
    ? `${config.provider.type}/${config.provider.model}`
    : t('agent.inheritPrimaryShort');

  return (
    <div className="xy-agent-page">
      <header className="xy-agent-header">
        <div className="xy-agent-header-copy">
          <p>{t('page.tasksDesc')}</p>
        </div>
        <div className="xy-agent-header-actions">
          <Button type="primary" icon={<PlusOutlined />} onClick={handleCreate}>
            {t('agent.create')}
          </Button>
        </div>
      </header>

      <section className="xy-agent-panel xy-agent-roster is-solo">
        <div className="xy-agent-panel-head">
          <h2>{t('agent.list')}</h2>
          <span className="xy-agent-panel-head-meta">{agents.length}</span>
        </div>
        <div className="xy-agent-panel-body">
          <Table
            rowKey="id"
            size="middle"
            pagination={false}
            tableLayout="fixed"
            className="xy-agent-table"
            dataSource={agents}
            locale={{ emptyText: <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t('agent.listEmpty', '暂无 Agent')} /> }}
            columns={[
              {
                title: t('agent.name'),
                key: 'name',
                width: '18%',
                align: 'center',
                render: (_: unknown, agent: AgentDefinitionInput) => (
                  <div className="xy-agent-name">
                    <div className="xy-agent-name-row">
                      <strong>{agent.name}</strong>
                      {agent.source === 'builtin' ? (
                        <Tag className="xy-agent-source-pill" color="purple">builtin</Tag>
                      ) : (
                        <Tag className="xy-agent-source-pill">{agent.source || 'project'}</Tag>
                      )}
                    </div>
                    <span>{agent.id}</span>
                  </div>
                ),
              },
              {
                title: t('agent.role'),
                dataIndex: 'role',
                key: 'role',
                width: '9%',
                align: 'center',
                render: (role: string) => (
                  <span className="xy-agent-role">{subAgentRoleLabel(role, locale)}</span>
                ),
              },
              {
                title: t('agent.status'),
                key: 'status',
                width: '8%',
                align: 'center',
                render: (_: unknown, agent: AgentDefinitionInput) => {
                  const member = memberByAgentId.get(agent.id);
                  return member
                    ? <Tag color={statusColor(member.status)}>{member.status}</Tag>
                    : <Tag>{t('status.idle')}</Tag>;
                },
              },
              {
                title: t('agent.model'),
                key: 'model',
                width: '18%',
                align: 'center',
                ellipsis: true,
                render: (_: unknown, agent: AgentDefinitionInput) => {
                  const member = memberByAgentId.get(agent.id);
                  const provider = member?.provider || agent.provider || t('agent.inherit');
                  const model = member?.model || agent.model || t('agent.inherit');
                  return (
                    <div className="xy-agent-model" title={`${provider}/${model}`}>
                      <span>{provider}/{model}</span>
                    </div>
                  );
                },
              },
              {
                title: t('agent.deepThinking'),
                key: 'deepThinking',
                width: '16%',
                align: 'center',
                render: (_: unknown, agent: AgentDefinitionInput) => {
                  const override = modelOverrides[agent.id] || {};
                  const member = memberByAgentId.get(agent.id);
                  const deepThinking = override.deepThinking ?? member?.deepThinking ?? false;
                  const effort = (override.reasoningEffort ||
                    member?.reasoningEffort ||
                    'high') as ReasoningEffort;
                  const index = agentThinkingIndex(deepThinking, effort);
                  const step = AGENT_THINKING_STEPS[index] || 'off';
                  const label =
                    step === 'off' ? t('common.off', '关') : t(`chat.reasoning.${step}`);
                  const isUltracode = deepThinking && effort === 'ultracode';
                  return (
                    <div className={`xy-agent-thinking${isUltracode ? ' is-ultracode' : ''}`}>
                      <span className="xy-agent-thinking-label">{label}</span>
                      <EffortSlider
                        value={index}
                        max={AGENT_THINKING_STEPS.length - 1}
                        isUltracode={isUltracode}
                        onChange={(next) => patchAgentModel(agent.id, agentThinkingFromIndex(next))}
                      />
                    </div>
                  );
                },
              },
              {
                title: t('agent.capabilities'),
                key: 'capabilities',
                width: '15%',
                align: 'center',
                render: (_: unknown, agent: AgentDefinitionInput) => {
                  const writable = agent.canWrite === true;
                  return (
                    <div className="xy-agent-caps">
                      <Tag color={writable ? 'orange' : 'blue'}>
                        {writable ? t('agent.writable') : t('agent.readonly')}
                      </Tag>
                      <Tag>{optionLabel(resolveVisiblePermissionMode(agent.permissionMode), locale)}</Tag>
                    </div>
                  );
                },
              },
              {
                title: t('agent.enabled'),
                key: 'enabled',
                width: '8%',
                align: 'center',
                render: (_: unknown, agent: AgentDefinitionInput) => (
                  <Switch
                    size="small"
                    checked={agent.enabled !== false}
                    onChange={(checked) => handleEnabledChange(agent, checked)}
                  />
                ),
              },
              {
                title: t('common.actions'),
                key: 'actions',
                width: '12%',
                align: 'center',
                render: (_: unknown, agent: AgentDefinitionInput) => (
                  <Space size={8} className="xy-agent-actions" wrap>
                    <Button
                      size="small"
                      icon={<EditOutlined />}
                      onClick={() => handleEdit(agent)}
                    >
                      {t('common.edit', '编辑')}
                    </Button>
                    {agent.source === 'builtin' ? null : (
                      <Popconfirm
                        title={t('agent.deleteConfirm')}
                        onConfirm={() => handleDelete(agent.id)}
                        okText={t('common.delete')}
                        cancelText={t('common.cancel')}
                      >
                        <Button size="small" danger icon={<DeleteOutlined />}>
                          {t('common.delete')}
                        </Button>
                      </Popconfirm>
                    )}
                  </Space>
                ),
              },
            ]}
          />
        </div>
      </section>

      <Drawer
        className="xy-agent-drawer"
        title={editingAgent ? t('agent.editTitle', undefined, editingAgent.name) : t('agent.newTitle')}
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
        width={560}
        destroyOnHidden
        extra={
          <Space>
            <Button onClick={() => setDrawerOpen(false)}>{t('common.cancel')}</Button>
            <Button type="primary" onClick={handleSave}>{t('common.save')}</Button>
          </Space>
        }
      >
        <Form
          form={form}
          layout="vertical"
          initialValues={{
            role: 'developer',
            memoryScope: 'session',
            permissionMode: 'default',
            defaultSandboxMode: 'workspace',
          }}
        >
          <div className="xy-agent-drawer-section">
            <h3>{t('agent.sectionIdentity')}</h3>
            <Form.Item name="id" label="ID" rules={[{ required: true }]}>
              <Input disabled={!!editingAgent} placeholder="developer" />
            </Form.Item>
            <Form.Item name="name" label={t('agent.name')} rules={[{ required: true }]}>
              <Input placeholder={t('agent.namePlaceholder')} />
            </Form.Item>
            <Form.Item name="role" label={t('agent.role')} extra={t('agent.roleHint')}>
              <Select
                disabled={editingAgent?.source === 'builtin'}
                options={ROLE_OPTIONS.map((role) => ({
                  value: role,
                  label: subAgentRoleLabel(role, locale),
                }))}
                showSearch
              />
            </Form.Item>
            <Form.Item name="description" label={t('agent.description')} rules={[{ required: true }]}>
              <Input placeholder={t('agent.descriptionPlaceholder')} />
            </Form.Item>
          </div>

          <div className="xy-agent-drawer-section">
            <h3>{t('agent.modelConfig')}</h3>
            <div className="xy-agent-inherit-bar">
              <div>
                <strong>{t('agent.inheritPrimary')}</strong>
                <div className="xy-agent-inherit-hint">{mainModelLine}</div>
              </div>
              <Switch checked={inheritModel} onChange={handleInheritChange} />
            </div>
            {!inheritModel ? (
              <>
                <Form.Item name="provider" label={t('agent.provider')} rules={[{ required: true }]}>
                  <ProviderSelect
                    providers={providers}
                    onProviderPicked={handleProviderChange}
                  />
                </Form.Item>
                <Form.Item name="model" label={t('agent.model')} rules={[{ required: true }]}>
                  <LiveModelSelect
                    providerType={watchedProvider || config?.provider?.type}
                    baseUrl={watchedBaseUrl || config?.provider?.baseUrl}
                    apiKey={watchedApiKey || config?.provider?.apiKey}
                    value={watchedModel}
                    allowClear={false}
                    showRefresh
                    enabled={!inheritModel && drawerOpen}
                    placeholder={t('agent.inheritPrimaryShort')}
                  />
                </Form.Item>
                <Form.Item name="baseUrl" label={t('agent.baseUrl')}>
                  <Input placeholder={t('agent.inherit')} />
                </Form.Item>
                <Form.Item name="apiKey" label={t('agent.apiKey')}>
                  <Input placeholder={t('agent.inherit')} autoComplete="off" />
                </Form.Item>
                <Form.Item name="maxTokens" label={t('agent.maxTokens')}>
                  <InputNumber style={{ width: '100%' }} min={256} max={128000} />
                </Form.Item>
                <Form.Item name="temperature" label={t('agent.temperature')}>
                  <InputNumber style={{ width: '100%' }} min={0} max={2} step={0.1} />
                </Form.Item>
              </>
            ) : (
              <Tag color="green">{t('agent.inheritPrimaryShort')}</Tag>
            )}
          </div>

          <div className="xy-agent-drawer-section">
            <h3>{t('agent.capabilityConfig')}</h3>
            <Form.Item name="permissionMode" label={t('agent.permissionMode')}>
              <Select
                options={PERMISSION_MODES.map((m) => ({ label: optionLabel(m, locale), value: m }))}
              />
            </Form.Item>
            <Form.Item name="defaultSandboxMode" label={t('agent.sandboxMode')}>
              <Select
                options={SANDBOX_MODES.map((m) => ({ label: optionLabel(m, locale), value: m }))}
              />
            </Form.Item>
            <Form.Item name="memoryScope" label={t('agent.memoryScope')}>
              <Select
                options={MEMORY_SCOPES.map((m) => ({ label: optionLabel(m, locale), value: m }))}
              />
            </Form.Item>
            <Form.Item
              name="tools"
              label={t('agent.tools')}
              extra={t('agent.toolsWriteHint', '是否可写由工具集决定：包含写入类工具即可写文件。')}
              rules={[{ required: true, type: 'array', min: 1 }]}
            >
              <Select
                mode="multiple"
                options={toolOptions}
                optionRender={(option) => renderToolSelectOption(option, locale)}
                optionFilterProp="title"
                placeholder={t('agent.selectTools')}
              />
            </Form.Item>
          </div>

          <div className="xy-agent-drawer-section">
            <h3>{t('agent.promptMarkdown')}</h3>
            <Form.Item name="prompt" rules={[{ required: true }]}>
              <TextArea rows={8} placeholder={t('agent.promptPlaceholder')} />
            </Form.Item>
          </div>
        </Form>
      </Drawer>
    </div>
  );
}

/** 被谁调用：AgentPanel。做什么：成员状态 → 颜色。 */
function statusColor(status: SubAgentTeamMember['status']): string {
  switch (status) {
    case 'running': return 'processing';
    case 'completed': return 'green';
    case 'failed': return 'red';
    case 'queued': return 'gold';
    default: return 'default';
  }
}

/** 被谁调用：AgentPanel。做什么：选项值本地化标签。 */
function optionLabel(value: string, locale: 'zh-CN' | 'en'): string {
  const label = OPTION_LABELS[value];
  return label ? label[locale === 'zh-CN' ? 0 : 1] : value;
}
