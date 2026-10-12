/**
 * 初始化向导「模型」步：配置供应商、主模型与各辅助任务槽位。
 */

import React, { useEffect, useMemo, useState } from 'react';
import {
  Button,
  Card,
  Col,
  Divider,
  Flex,
  Form,
  Input,
  InputNumber,
  Modal,
  Row,
  Segmented,
  Select,
  Slider,
  Switch,
  Tag,
  Typography,
  message,
} from 'antd';
import { EditOutlined } from '@ant-design/icons';

import { useT } from '../i18n';
import { ModelSelect, ProviderSelect } from '../components/model';
import { useProviderModels } from '../modelCatalog';
import type { InitDraft, InitMeta, InitProviderDef } from './initApi';
import {
  MODEL_SLOTS,
  type AgentModelPatch,
  type AuxModelTask,
  type ModelSlotId,
  formatModelLine,
  isAuxInherited,
  readAuxModel,
} from './modelSlots';

const { Title, Paragraph, Text } = Typography;

interface ModelsStepProps {
  meta: InitMeta;
  draft: InitDraft;
  providerDef: InitProviderDef | null;
  patchConfig: (p: InitDraft['config']) => void;
  setDraft: React.Dispatch<React.SetStateAction<InitDraft | null>>;
  onProviderChange: (type: string) => void;
}

/** 被谁调用：InitWizard。做什么：provider/model 配置步。 */
export default function ModelsStep(props: ModelsStepProps) {
  const { meta, draft, providerDef, patchConfig, setDraft, onProviderChange } = props;
  const tr = useT();
  const p = draft.config.provider!;
  const auxiliaryModels = (draft.config.modelRouting?.auxiliaryModels || {}) as Record<string, AgentModelPatch>;

  const [editingSlot, setEditingSlot] = useState<ModelSlotId | null>(null);
  const [modalTab, setModalTab] = useState<'provider' | 'model'>('provider');
  const [inheritAux, setInheritAux] = useState(true);

  const fetchParams = useMemo(() => {
    if (!editingSlot) return null;
    if (editingSlot === 'main') {
      return {
        providerType: p.type || 'openai',
        baseUrl: p.baseUrl,
        apiKey: p.apiKey,
        planId: draft.providerPlanId,
        currentModel: p.model,
      };
    }
    const aux = readAuxModel(auxiliaryModels, editingSlot);
    return {
      providerType: aux.provider || p.type || 'openai',
      baseUrl: aux.baseUrl || p.baseUrl,
      apiKey: aux.apiKey || p.apiKey,
      currentModel: aux.model,
    };
  }, [auxiliaryModels, draft.providerPlanId, editingSlot, p.apiKey, p.baseUrl, p.model, p.type]);

  const {
    models: modalModels,
    loading: loadingModels,
    error: modelsError,
    refresh: refreshModels,
  } = useProviderModels({
    providerType: fetchParams?.providerType,
    baseUrl: fetchParams?.baseUrl,
    apiKey: fetchParams?.apiKey,
    planId: fetchParams && 'planId' in fetchParams ? fetchParams.planId : undefined,
    currentModel: fetchParams?.currentModel,
    enabled: Boolean(editingSlot && fetchParams),
  });

  useEffect(() => {
    if (modelsError) message.warning(modelsError);
  }, [modelsError]);

  const openEditor = (slot: ModelSlotId) => {
    setModalTab('provider');
    setInheritAux(slot !== 'main' && isAuxInherited(auxiliaryModels, slot));
    setEditingSlot(slot);
  };

  const closeEditor = () => {
    setEditingSlot(null);
  };

  const patchAux = (task: AuxModelTask, patch: AgentModelPatch | null) => {
    const next = { ...auxiliaryModels };
    if (patch === null) {
      delete next[task];
    } else {
      next[task] = { ...next[task], ...patch };
    }
    patchConfig({
      modelRouting: {
        ...draft.config.modelRouting,
        auxiliaryModels: next,
      },
    });
  };

  const clearAux = (task: AuxModelTask) => {
    const next = { ...auxiliaryModels };
    delete next[task];
    patchConfig({
      modelRouting: {
        ...draft.config.modelRouting,
        auxiliaryModels: next,
      },
    });
  };

  const slotSummary = (slot: (typeof MODEL_SLOTS)[number]) => {
    if (slot.kind === 'main') {
      return formatModelLine(p.type, p.model) || tr('init.models.notConfigured');
    }
    if (isAuxInherited(auxiliaryModels, slot.id as AuxModelTask)) {
      return `${tr('init.models.inheritTag')} · ${formatModelLine(p.type, p.model)}`;
    }
    const aux = readAuxModel(auxiliaryModels, slot.id as AuxModelTask);
    return formatModelLine(aux.provider || p.type, aux.model) || tr('init.models.customNoModel');
  };

  const slotTag = (slot: (typeof MODEL_SLOTS)[number]) => {
    if (slot.kind === 'main') {
      return <Tag color="cyan">{tr('init.models.slot.main.tag')}</Tag>;
    }
    return isAuxInherited(auxiliaryModels, slot.id as AuxModelTask)
      ? <Tag color="green">{tr('init.models.inheritTag')}</Tag>
      : <Tag color="blue">{tr('init.models.customTag')}</Tag>;
  };

  const renderProviderForm = () => {
    if (editingSlot === 'main') {
      const selectedPlan = providerDef?.plans.find((plan) => plan.id === draft.providerPlanId) || providerDef?.plans[0];
      const hasPlans = !!(providerDef && providerDef.plans.length > 1);
      const needsApiKey = providerDef?.needsApiKey !== false;

      return (
        <Form layout="vertical" className="xy-init-form-grid">
          <Row gutter={16}>
            <Col xs={24} md={hasPlans ? 12 : 24}>
              <Form.Item label={tr('agent.provider')} required>
                <ProviderSelect
                  providers={meta.providers}
                  value={p.type}
                  onChange={onProviderChange}
                />
              </Form.Item>
            </Col>
            {hasPlans ? (
              <Col xs={24} md={12}>
                <Form.Item label={tr('init.provider.plan')}>
                  <Select
                    value={draft.providerPlanId}
                    onChange={(id) => {
                      const plan = providerDef!.plans.find((x) => x.id === id);
                      setDraft((prev) => prev ? {
                        ...prev,
                        providerPlanId: id,
                        config: {
                          ...prev.config,
                          provider: {
                            ...prev.config.provider!,
                            baseUrl: plan?.baseUrl,
                            model: '',
                          },
                        },
                      } : prev);
                    }}
                    options={providerDef!.plans.map((pl) => ({ value: pl.id, label: pl.label }))}
                  />
                </Form.Item>
              </Col>
            ) : null}
          </Row>
          {selectedPlan ? (
            <div className="xy-init-inline-note">
              <Tag color="cyan">{selectedPlan.protocol}</Tag>
              <span>{selectedPlan.description}</span>
            </div>
          ) : null}
          <Row gutter={16}>
            <Col xs={24} md={needsApiKey ? 12 : 24}>
              <Form.Item label={tr('agent.baseUrl')}>
                <Input value={p.baseUrl || ''} onChange={(e) => patchConfig({ provider: { ...p, baseUrl: e.target.value } })} placeholder="https://api.example.com/v1" />
              </Form.Item>
            </Col>
            {needsApiKey ? (
              <Col xs={24} md={12}>
                <Form.Item label={tr('agent.apiKey')} required>
                  <Input value={p.apiKey || ''} onChange={(e) => patchConfig({ provider: { ...p, apiKey: e.target.value } })} placeholder="sk-..." autoComplete="off" />
                </Form.Item>
              </Col>
            ) : null}
          </Row>
        </Form>
      );
    }

    const task = editingSlot as AuxModelTask;
    const aux = readAuxModel(auxiliaryModels, task);
    const auxProviderDef = meta.providers.find((item) => item.type === (aux.provider || p.type));
    const needsApiKey = auxProviderDef?.needsApiKey !== false;

    return (
      <Form layout="vertical" className="xy-init-form-grid">
        <div className="xy-init-model-inherit-bar">
          <div>
            <strong>{tr('init.models.inheritLabel')}</strong>
            <div><Text type="secondary">{tr('init.models.inheritHint')}</Text></div>
          </div>
          <Switch
            checked={inheritAux}
            onChange={(checked) => {
              setInheritAux(checked);
              if (checked) clearAux(task);
            }}
          />
        </div>
        {!inheritAux ? (
          <>
            <Row gutter={16}>
              <Col xs={24} md={12}>
                <Form.Item label={tr('agent.provider')}>
                  <ProviderSelect
                    providers={meta.providers}
                    value={aux.provider || p.type}
                    onProviderPicked={(provider) => {
                      const def = meta.providers.find((item) => item.type === provider);
                      patchAux(task, {
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
                    onChange={(e) => patchAux(task, { baseUrl: e.target.value || undefined })}
                    placeholder={p.baseUrl || 'https://api.example.com/v1'}
                  />
                </Form.Item>
              </Col>
            </Row>
            {needsApiKey ? (
              <Form.Item label={tr('agent.apiKey')}>
                <Input
                  value={aux.apiKey || ''}
                  onChange={(e) => patchAux(task, { apiKey: e.target.value || undefined })}
                  placeholder={tr('init.models.inheritApiKeyPh')}
                  autoComplete="off"
                />
              </Form.Item>
            ) : null}
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

  const renderModelForm = () => {
    if (editingSlot === 'main') {
      return (
        <Form layout="vertical" className="xy-init-form-grid">
          <Form.Item label={tr('init.model.mainModel')} required>
            <ModelSelect
              models={modalModels}
              loading={loadingModels}
              refreshing={loadingModels}
              value={p.model}
              onChange={(m) => patchConfig({ provider: { ...p, model: m || p.model } })}
              onRefresh={refreshModels}
              refreshLabel={tr('init.model.refreshModels')}
            />
          </Form.Item>
          <Row gutter={16}>
            <Col span={24}>
              <Form.Item label={tr('agent.temperature')}>
                <Flex gap={12} align="center">
                  <Slider
                    style={{ flex: 1, margin: '0 6px' }}
                    min={0}
                    max={2}
                    step={0.1}
                    value={p.temperature ?? 0.7}
                    onChange={(v) => patchConfig({ provider: { ...p, temperature: v } })}
                  />
                  <InputNumber
                    style={{ width: 96 }}
                    min={0}
                    max={2}
                    step={0.1}
                    value={p.temperature ?? 0.7}
                    onChange={(v) => patchConfig({ provider: { ...p, temperature: v ?? 0.7 } })}
                  />
                </Flex>
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={16}>
            <Col xs={24} md={12}>
              <Form.Item label={tr('agent.maxTokens')}>
                <InputNumber className="xy-init-input-full" min={256} max={128000} value={p.maxTokens ?? 4096} onChange={(v) => patchConfig({ provider: { ...p, maxTokens: v ?? 4096 } })} />
              </Form.Item>
            </Col>
            <Col xs={24} md={12}>
              <Form.Item label={tr('init.model.maxContext')}>
                <InputNumber className="xy-init-input-full" min={8000} max={2000000} value={draft.config.maxContextTokens ?? 128000} onChange={(v) => patchConfig({ maxContextTokens: v ?? 128000 })} />
              </Form.Item>
            </Col>
          </Row>
          <Divider>{tr('init.model.retrySection')}</Divider>
          <Row gutter={16}>
            <Col xs={24} sm={8}>
              <Form.Item label={tr('init.model.maxAttempts')}>
                <InputNumber className="xy-init-input-full" min={1} max={10} value={p.retry?.maxAttempts ?? 5} onChange={(v) => patchConfig({ provider: { ...p, retry: { ...p.retry, maxAttempts: v ?? 5 } } })} />
              </Form.Item>
            </Col>
            <Col xs={24} sm={8}>
              <Form.Item label={tr('init.model.baseDelay')}>
                <InputNumber className="xy-init-input-full" min={0} max={60000} step={100} value={p.retry?.baseDelayMs ?? 300} onChange={(v) => patchConfig({ provider: { ...p, retry: { ...p.retry, baseDelayMs: v ?? 300 } } })} />
              </Form.Item>
            </Col>
            <Col xs={24} sm={8}>
              <Form.Item label={tr('init.model.timeout')}>
                <InputNumber className="xy-init-input-full" min={5000} max={600000} step={1000} value={p.retry?.timeoutMs ?? 120000} onChange={(v) => patchConfig({ provider: { ...p, retry: { ...p.retry, timeoutMs: v ?? 120000 } } })} />
              </Form.Item>
            </Col>
          </Row>
        </Form>
      );
    }

    if (!editingSlot || inheritAux) {
      return (
        <div className="xy-init-inline-note">
          <Tag color="green">{tr('init.models.inheritTag')}</Tag>
          <span>{tr('init.models.inheritParamsHint')}</span>
        </div>
      );
    }

    const task = editingSlot as AuxModelTask;
    const aux = readAuxModel(auxiliaryModels, task);

    return (
      <Form layout="vertical" className="xy-init-form-grid">
        <Form.Item label={tr('init.model.mainModel')}>
          <ModelSelect
            models={modalModels}
            loading={loadingModels}
            refreshing={loadingModels}
            value={aux.model}
            onChange={(m) => patchAux(task, { model: m || undefined })}
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
                onChange={(v) => patchAux(task, { temperature: v ?? undefined })}
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
                onChange={(v) => patchAux(task, { maxTokens: v ?? undefined })}
              />
            </Form.Item>
          </Col>
        </Row>
      </Form>
    );
  };

  const editingSlotDef = editingSlot ? MODEL_SLOTS.find((slot) => slot.id === editingSlot) : null;

  return (
    <>
      <Title level={4}>{tr('init.models.title')}</Title>
      <Paragraph type="secondary">{tr('init.models.desc')}</Paragraph>

      <div className="xy-init-model-grid">
        {MODEL_SLOTS.map((slot) => (
          <Card
            key={slot.id}
            hoverable
            className={`xy-init-model-card${slot.kind === 'main' ? ' is-main' : ''}${slot.kind === 'auxiliary' && !isAuxInherited(auxiliaryModels, slot.id as AuxModelTask) ? ' is-custom' : ''}`}
            style={slot.kind === 'main' ? { gridColumn: '1 / -1' } : undefined}
            onClick={() => openEditor(slot.id)}
          >
            <div className="xy-init-model-card-head">
              <div>
                <strong>{tr(slot.nameKey)}</strong>
                <p>{tr(slot.hintKey)}</p>
              </div>
              {slotTag(slot)}
            </div>
            <div className="xy-init-model-card-body">
              <span className="xy-init-model-card-line">{slotSummary(slot)}</span>
            </div>
            <div className="xy-init-model-card-foot">
              <span>{tr('init.models.edit')}</span>
              <EditOutlined />
            </div>
          </Card>
        ))}
      </div>

      <Modal
        className="xy-init-model-modal"
        rootClassName="xy-init-model-modal-root"
        open={!!editingSlot}
        title={editingSlotDef ? tr(editingSlotDef.nameKey) : ''}
        width={760}
        onCancel={closeEditor}
        footer={[
          <Button key="close" type="primary" onClick={closeEditor}>{tr('init.done')}</Button>,
        ]}
        destroyOnHidden
      >
        <Segmented
          block
          size="large"
          className="xy-init-model-segmented"
          value={modalTab}
          onChange={(value) => setModalTab(value as 'provider' | 'model')}
          options={[
            { label: tr('init.models.providerTab'), value: 'provider' },
            { label: tr('init.models.modelTab'), value: 'model' },
          ]}
        />
        <div className="xy-init-segment-panel" role="tabpanel">
          {modalTab === 'provider' ? renderProviderForm() : renderModelForm()}
        </div>
      </Modal>
    </>
  );
}
