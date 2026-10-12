/**
 * Shared provider / model pickers used by Agent 编辑、配置中心、init 向导。
 */

import React, { useMemo } from 'react';
import { Button, Col, Row, Select, Typography, type SelectProps } from 'antd';

const { Text } = Typography;

/** Normalized provider row for UI selects (gateway catalog or init meta). */
export interface ProviderCatalogItem {
  type: string;
  name?: string;
  label?: string;
  description?: string;
  defaultBaseUrl?: string;
}

/** Select 展示名：label → name → type。 */
export function providerDisplayName(item: ProviderCatalogItem): string {
  return item.label || item.name || item.type;
}

/** 目录 → Ant Design Select options（带 description）。 */
export function providerSelectOptions(providers: ProviderCatalogItem[]) {
  return providers.map((item) => ({
    value: item.type,
    label: providerDisplayName(item),
    desc: item.description,
  }));
}

/** 模型 id 列表 → Select options。 */
export function modelSelectOptions(models: string[]) {
  return models.filter(Boolean).map((id) => ({ value: id, label: id }));
}

/** 按 type 找目录项；type 空则 undefined。 */
export function findProvider(
  providers: ProviderCatalogItem[],
  type: string | undefined,
): ProviderCatalogItem | undefined {
  if (!type) return undefined;
  return providers.find((item) => item.type === type);
}

type ProviderSelectProps = Omit<SelectProps, 'options' | 'mode'> & {
  providers: ProviderCatalogItem[];
  onProviderPicked?: (type: string, entry: ProviderCatalogItem | undefined) => void;
};

/** Provider type dropdown with search; optional description under each option. */
export function ProviderSelect(props: ProviderSelectProps) {
  const { providers, onProviderPicked, onChange, ...rest } = props;
  const options = useMemo(() => providerSelectOptions(providers), [providers]);
  const showDesc = providers.some((item) => !!item.description);

  return (
    <Select
      showSearch
      optionFilterProp="label"
      options={options}
      optionRender={
        showDesc
          ? (opt) => (
              <div>
                <div>{opt.label}</div>
                {opt.data.desc ? (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {opt.data.desc}
                  </Text>
                ) : null}
              </div>
            )
          : undefined
      }
      onChange={(value, option) => {
        onChange?.(value, option);
        if (typeof value === 'string') {
          onProviderPicked?.(value, findProvider(providers, value));
        }
      }}
      {...rest}
    />
  );
}

type ModelSelectProps = Omit<SelectProps, 'options' | 'mode'> & {
  models: string[];
  /** When set, renders a refresh button beside the select. */
  onRefresh?: () => void;
  refreshLabel?: string;
  refreshing?: boolean;
};

/** Model id dropdown; optional refresh action for live model lists. */
export function ModelSelect(props: ModelSelectProps) {
  const {
    models,
    onRefresh,
    refreshLabel,
    refreshing,
    loading,
    style,
    ...rest
  } = props;
  const options = useMemo(() => modelSelectOptions(models), [models]);
  const select = (
    <Select
      showSearch
      allowClear
      optionFilterProp="label"
      options={options}
      loading={loading || refreshing}
      style={{ width: '100%', ...style }}
      {...rest}
    />
  );

  if (!onRefresh) return select;

  return (
    <Row gutter={12} wrap={false}>
      <Col flex="auto">{select}</Col>
      <Col flex="0 0 auto">
        <Button loading={refreshing || loading} onClick={onRefresh}>
          {refreshLabel || 'Refresh'}
        </Button>
      </Col>
    </Row>
  );
}
