/**
 * PermissionDialog - 权限请求卡片
 *
 * 用途：聊天流中的内嵌权限审批卡片，不额外开启 modal，避免打断阅读上下文。
 * 展示工具名、审批原因和参数 JSON，提供允许/本次会话都允许/拒绝/本次会话都拒绝四种操作。
 */

import React from 'react';
import { Button, Space, Tag, Typography } from 'antd';
import { LockOutlined } from '@ant-design/icons';
import type { PendingPermission } from '../../stores/chatStore';
import { useI18n } from '../../i18n';

const { Text } = Typography;

interface Props {
  pending: PendingPermission;
  onRespond: (allowed: boolean, remember?: boolean) => void;
}

// 聊天流中的内嵌权限卡片，不额外开启 modal，避免打断阅读上下文。
export default function PermissionDialog({ pending, onRespond }: Props) {
  const { t } = useI18n();
  // 弹窗只负责展示待审批内容和四个决策按钮。
  // 这里不维护任何本地状态，所有决策都直接回传给上层 store。
  return (
    <div className="xy-permission-card">
      <div className="xy-permission-head">
        <div className="xy-permission-icon">
          <LockOutlined />
        </div>
        <div className="xy-permission-title">
          <Text strong>{t('permission.requestTitle')}</Text>
          <Text type="secondary">{t('permission.requestSubtitle')}</Text>
        </div>
        <Tag color="gold">{pending.toolName}</Tag>
      </div>

      {/* reason 来自后端生成的审批解释，帮助用户快速判断风险。 */}
      <div className="xy-permission-reason">{pending.reason}</div>

      {/* 参数区保持原样 JSON，便于用户直接核对调用边界。 */}
      <pre className="xy-pre xy-permission-pre">
        {/* 直接展示参数 JSON，方便用户确认调用内容。 */}
        {JSON.stringify(pending.args, null, 2)}
      </pre>

      <Space wrap className="xy-permission-actions">
        {/* 四个操作对应 allow / allow+session / deny / deny+session。 */}
        <Button type="primary" onClick={() => onRespond(true)}>
          {t('permission.allow')}
        </Button>
        {/* remember 在允许/拒绝时都会写入会话名单。 */}
        <Button onClick={() => onRespond(true, true)}>
          {t('permission.allowSession')}
        </Button>
        {/* 拒绝按钮保留为危险态，降低误点概率。 */}
        <Button danger onClick={() => onRespond(false)}>
          {t('permission.deny')}
        </Button>
        <Button danger onClick={() => onRespond(false, true)}>
          {t('permission.denySession')}
        </Button>
      </Space>
    </div>
  );
}
