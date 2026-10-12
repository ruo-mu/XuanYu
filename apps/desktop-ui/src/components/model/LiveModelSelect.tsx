/**
 * 带缓存的实时模型下拉：设置 / Agent 编辑共用。
 */

import React from 'react';
import { Typography, type SelectProps } from 'antd';

import { useProviderModels } from '../../modelCatalog';
import { ModelSelect } from './ProviderModelSelect';

const { Text } = Typography;

type LiveModelSelectProps = Omit<SelectProps, 'options' | 'mode' | 'loading'> & {
  providerType?: string;
  baseUrl?: string;
  apiKey?: string;
  protocol?: string;
  planId?: string;
  enabled?: boolean;
  refreshLabel?: string;
  showRefresh?: boolean;
};

/** 被谁调用：ConfigPanel / AgentPanel。做什么：拉模型列表并渲染 ModelSelect。 */
export function LiveModelSelect(props: LiveModelSelectProps) {
  const {
    providerType,
    baseUrl,
    apiKey,
    protocol,
    planId,
    enabled = true,
    refreshLabel,
    showRefresh = true,
    value,
    ...rest
  } = props;

  const currentModel = typeof value === 'string' ? value : undefined;
  const { models, loading, error, refresh } = useProviderModels({
    providerType,
    baseUrl,
    apiKey,
    protocol,
    planId,
    enabled,
    currentModel,
  });

  return (
    <div>
      <ModelSelect
        models={models}
        loading={loading}
        refreshing={loading}
        value={value}
        onRefresh={showRefresh ? refresh : undefined}
        refreshLabel={refreshLabel}
        notFoundContent={loading ? undefined : (error || undefined)}
        {...rest}
      />
      {!loading && error ? (
        <Text type="danger" style={{ display: 'block', marginTop: 6, fontSize: 12 }}>
          {error}
        </Text>
      ) : null}
    </div>
  );
}
