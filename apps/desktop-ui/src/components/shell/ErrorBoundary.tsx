/**
 * 渲染错误兜底：子树抛错时显示提示，避免整页白屏。
 */

import React from 'react';

import { Typography } from 'antd';

import { WarningOutlined } from '@ant-design/icons';
import { useI18n } from '../../i18n';

const { Text } = Typography;

interface Props {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}

interface InternalProps extends Props {
  renderErrorLabel: string;
  unknownErrorLabel: string;
}

interface State {
  hasError: boolean;
  error?: Error;
}

/** 用于兜底渲染异常，避免整页白屏。 */
class ErrorBoundaryInner extends React.Component<InternalProps, State> {
  /** 初始化 hasError=false。 */
  constructor(props: InternalProps) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(error: Error): State {
    // 渲染阶段抛错后只切换到 fallback，不在这里做副作用。
    return { hasError: true, error };
  }

  /** 渲染错误时打控制台日志；不做二次上报。 */
  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    // 仅记录控制台日志，不在这里尝试二次上报。
    console.error('[ErrorBoundary] Rendering error:', error.message, errorInfo);
  }

  /** 有错则渲染 fallback 或默认错误条，否则渲染 children。 */
  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        // 调用方提供了定制 fallback 时，优先使用外部视图。
        return this.props.fallback;
      }
      return (
        <div style={{
          padding: '8px 12px',
          margin: '4px 0',
          background: 'color-mix(in srgb, var(--xy-danger) 10%, var(--xy-bg-elevated))',
          border: '1px solid color-mix(in srgb, var(--xy-danger) 30%, var(--xy-border))',
          borderRadius: 6,
          fontSize: 12,
        }}>
          {/* 默认错误态只暴露 message，避免把内部堆栈直接泄露到界面。 */}
          <Text type="danger">
            <WarningOutlined style={{ marginRight: 4 }} />
            {this.props.renderErrorLabel}: {this.state.error?.message || this.props.unknownErrorLabel}
          </Text>
        </div>
      );
    }
    return this.props.children;
  }
}

/** 用 i18n 文案包装内部 class 错误边界。 */
export default function ErrorBoundary(props: Props) {
  const { t } = useI18n();
  return <ErrorBoundaryInner {...props} renderErrorLabel={t('error.render')} unknownErrorLabel={t('error.unknown')} />;
}
