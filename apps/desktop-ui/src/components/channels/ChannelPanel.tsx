/**
 * 频道页：管理 IM 连接、扫码登录、会话列表与频道对话记录。
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Button,
  Form,
  Input,
  Modal,
  Switch,
  message,
} from 'antd';
import {
  ApiOutlined,
  DeleteOutlined,
  DisconnectOutlined,
  LinkOutlined,
  MenuFoldOutlined,
  MenuUnfoldOutlined,
  PlusOutlined,
  QrcodeOutlined,
  ReloadOutlined,
  SettingOutlined,
} from '@ant-design/icons';
import { Icon } from '@iconify/react';
import { useWs } from '../../App';
import { useI18n, useLocaleText } from '../../i18n';
import {
  channelTypeFromLocation,
  navigateToPage,
} from '../../navigation/pageRegistry';
import type { SessionInfo } from '../../protocol';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import { AdminPage } from '../shell/PanelShell';
import { ChannelQrBoard } from './ChannelQrBoard';
import { ChannelTranscript, type ChannelChatMessage } from './ChannelTranscript';
import { CHANNEL_CATALOG } from './channelCatalog';
import { channelBrandIcon } from './channelIcons';

type ChannelAvailability = 'supported' | 'experimental' | 'unsupported';
type ChannelStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

interface ChannelAdapterInfo {
  type: string;
  label: string;
  description?: string;
  setupHint?: string;
  availability: ChannelAvailability;
}

interface ChannelConnection {
  type: string;
  label: string;
  availability: ChannelAvailability;
  status: ChannelStatus;
  error?: string;
  messageCount: number;
  config: {
    type: string;
    name: string;
    enabled: boolean;
    connectionId?: string;
    token?: string;
    tenantId?: string;
    mentionOnly?: boolean;
    allowlist?: string[];
    extra?: Record<string, string>;
  };
}

interface ChannelFormValues {
  name: string;
  tenantId: string;
  token?: string;
  mentionOnly?: boolean;
  allowlist?: string;
  proactiveChannelId?: string;
  proactiveEnabled?: boolean;
  appId?: string;
  accountId?: string;
  sandbox?: boolean;
  verificationToken?: string;
  webhookPort?: string;
  phoneNumberId?: string;
  verifyToken?: string;
  appSecret?: string;
  channelSecret?: string;
  /** LINE Developers 控制台的 Channel ID */
  channelId?: string;
  /** Telegram HTTP 代理，如 http://127.0.0.1:7890 */
  proxyUrl?: string;
}

/** 有官方账号/应用 ID 的平台：连接名直接用该 ID，不手填显示名 */
const OFFICIAL_IDENTITY_TYPES = new Set([
  'wechat',
  'qq',
  'feishu',
  'telegram',
  'whatsapp',
  'line',
]);

/** 被谁调用：ChannelPanel。做什么：判断是否走官方身份登录。 */
function usesOfficialIdentity(type: string): boolean {
  return OFFICIAL_IDENTITY_TYPES.has(type);
}

/** 从凭证推导连接名（与微信 ilink_bot_id 同一套路） */
function resolveConnectionName(
  type: string,
  values: Partial<ChannelFormValues>,
): string {
  switch (type) {
    case 'wechat': {
      const id = values.accountId?.trim();
      if (!id) throw new Error('请填写 Bot ID');
      return id;
    }
    case 'qq':
    case 'feishu': {
      const id = values.appId?.trim();
      if (!id) throw new Error('请填写 App ID');
      return id;
    }
    case 'whatsapp': {
      const id = values.phoneNumberId?.trim();
      if (!id) throw new Error('请填写 Phone Number ID');
      return id;
    }
    case 'telegram': {
      const token = values.token?.trim() || '';
      if (token) {
        const botId = token.split(':')[0]?.trim() || '';
        if (!/^\d+$/.test(botId)) {
          throw new Error('Bot Token 格式无效（应为 数字ID:密钥）');
        }
        return botId;
      }
      // 编辑时 Token 不回填，沿用已保存的连接名（即 Bot ID）
      const existing = values.name?.trim();
      if (existing) return existing;
      throw new Error('请填写 Bot Token');
    }
    case 'line': {
      const id = values.channelId?.trim();
      if (!id) throw new Error('请填写 Channel ID');
      return id;
    }
    default: {
      const name = values.name?.trim();
      if (!name) throw new Error('请填写显示名称');
      return name;
    }
  }
}

/**
 * Product catalog lives in channelCatalog.ts (shared with AppSidebar).
 */

/** 被谁调用：ChannelPanel。做什么：连接状态文案。 */
function statusLabel(
  status: ChannelStatus,
  lt: (zh: string, en: string, ...args: Array<string | number>) => string,
): string {
  switch (status) {
    case 'disconnected':
      return lt('未连接', 'Disconnected');
    case 'connecting':
      return lt('连接中', 'Connecting');
    case 'connected':
      return lt('已连接', 'Connected');
    case 'error':
      return lt('异常', 'Error');
    default:
      return status;
  }
}

/** 连接唯一键：优先 connectionId，否则 type:name。 */
function keyOf(item: ChannelConnection): string {
  return item.config.connectionId || `${item.config.type}:${item.config.name}`;
}

/** 会话列表展示名：去掉「qq ·」「Channel 」等前缀 */
function displaySessionName(name: string): string {
  const trimmed = name.trim();
  const stripped = trimmed
    .replace(/^Channel\s+/i, '')
    .replace(/^(wechat|qq|telegram|discord|feishu|dingtalk|whatsapp|line)\s*[·•\-–—:]\s*/i, '')
    .trim();
  return stripped || trimmed;
}

/** 被谁调用：ChannelPanel。做什么：是否实例级会话条目。 */
function isInstanceSession(
  session: SessionInfo,
  channelType: string,
  instance: ChannelConnection,
): boolean {
  const meta = session.metadata || {};
  if (meta.source !== 'channel' || meta.channelType !== channelType) return false;
  const instanceKey = keyOf(instance);
  if (typeof meta.connectionId === 'string' && meta.connectionId) {
    return meta.connectionId === instanceKey;
  }
  if (typeof meta.channelName === 'string' && meta.channelName) {
    return meta.channelName === instance.config.name;
  }
  return false;
}

/** 频道工作台：连接列表 + 当前会话对话。 */
export default function ChannelPanel() {
  const { send, addHandler, connected } = useWs();
  const { locale } = useI18n();
  const lt = useLocaleText();
  const ltRef = useRef(lt);
  ltRef.current = lt;
  const initSession = useChatStore((s) => s.initSession);
  const setActiveSession = useChatStore((s) => s.setActiveSession);
  const addSession = useSessionStore((s) => s.addSession);

  const [adapters, setAdapters] = useState<ChannelAdapterInfo[]>([]);
  const [connections, setConnections] = useState<ChannelConnection[]>([]);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeType, setActiveType] = useState<string>(() => channelTypeFromLocation());
  const [configOpen, setConfigOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [qrConnecting, setQrConnecting] = useState(false);
  const [qrPayload, setQrPayload] = useState<string | null>(null);
  const [qrKey, setQrKey] = useState<string | null>(null);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [selectedInstanceKey, setSelectedInstanceKey] = useState<string | null>(null);
  const [editingInstanceKey, setEditingInstanceKey] = useState<string | null>(null);
  const [historyMessages, setHistoryMessages] = useState<ChannelChatMessage[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [streamingText, setStreamingText] = useState('');
  const [testing, setTesting] = useState(false);
  const [manualTestOk, setManualTestOk] = useState(false);
  const [instancesCollapsed, setInstancesCollapsed] = useState(false);
  const [sessionsCollapsed, setSessionsCollapsed] = useState(false);
  const [form] = Form.useForm<ChannelFormValues>();

  const selectedSessionRef = useRef<string | null>(null);
  const qrKeyRef = useRef<string | null>(null);
  const autoQrStartedRef = useRef<string | null>(null);
  const keysBeforeDraftRef = useRef<Set<string>>(new Set());
  /** 手动测试 / 保存 / 列表重连 的等待键，避免和扫码成功互相抢 toast */
  const pendingWatchRef = useRef<{
    key: string;
    kind: 'test' | 'save' | 'reconnect';
    previousKey?: string;
  } | null>(null);

  const appendHistoryMessage = useCallback((incoming: ChannelChatMessage) => {
    setHistoryMessages((prev) => {
      const content = incoming.content || '';
      const duplicate = prev.some(
        (item) =>
          item.role === incoming.role &&
          item.content === content &&
          Math.abs((item.timestamp || 0) - (incoming.timestamp || 0)) < 5000,
      );
      if (duplicate) return prev;
      return [...prev, incoming];
    });
  }, []);

  const refresh = useCallback(() => {
    send('channel.adapters.list', {});
    send('channel.connections.list', {});
    send('session.list', { limit: 200 });
  }, [send]);

  useEffect(() => {
    qrKeyRef.current = qrKey;
  }, [qrKey]);

  useEffect(
    () =>
      addHandler((msg) => {
        if (msg.type === 'channel.adapters.list') {
          setAdapters((msg.payload as { adapters?: ChannelAdapterInfo[] }).adapters || []);
        }
        if (msg.type === 'channel.connections.list') {
          const next = (msg.payload as { connections?: ChannelConnection[] }).connections || [];
          setConnections(next);
          const pending = pendingWatchRef.current;
          if (!pending) return;
          const current = next.find((item) => keyOf(item) === pending.key);
          if (!current) return;
          if (current.status === 'connected') {
            const kind = pending.kind;
            const previousKey = pending.previousKey;
            pendingWatchRef.current = null;
            setTesting(false);
            setSubmitting(false);
            if (kind === 'test') {
              setManualTestOk(true);
              message.success(ltRef.current('测试连接通过', 'Connection test passed'));
              return;
            }
            if (kind === 'save') {
              if (previousKey) {
                send('channel.connection.remove', { key: previousKey });
              }
              setSelectedInstanceKey(pending.key);
              setConfigOpen(false);
              setEditingInstanceKey(null);
              setManualTestOk(false);
              message.success(ltRef.current('已保存并连接', 'Saved and connected'));
              return;
            }
            if (kind === 'reconnect') {
              setQrConnecting(false);
              setQrPayload(null);
              setConfigOpen(false);
              message.success(ltRef.current('连接成功', 'Connected'));
            }
          } else if (current.status === 'error') {
            pendingWatchRef.current = null;
            setTesting(false);
            setSubmitting(false);
            setManualTestOk(false);
            message.error(
              current.error ||
                (pending.kind === 'test'
                  ? ltRef.current('测试连接失败', 'Connection test failed')
                  : ltRef.current('连接失败', 'Connection failed')),
            );
          }
        }
        if (msg.type === 'channel.connection.updated') {
          // 等待 connections.list 的最终状态再提示，这里只解除提交锁的兜底由 list 处理
        }
        if (msg.type === 'channel.qr') {
          const payload = msg.payload as { key?: string; qrUrl?: string };
          if (payload.key) setQrKey(payload.key);
          if (payload.qrUrl) {
            setQrPayload(payload.qrUrl);
            setQrConnecting(true);
          }
        }
        if (msg.type === 'channel.qr.result') {
          const payload = msg.payload as {
            draftId?: string;
            ok?: boolean;
            key?: string;
            error?: string;
          };
          const watching = qrKeyRef.current;
          if (payload.draftId && watching && payload.draftId !== watching) return;
          // 扫码结果独占反馈，清掉 pending，避免随后 list 再弹「连接成功」
          pendingWatchRef.current = null;
          if (payload.ok && payload.key) {
            setQrKey(null);
            setSelectedInstanceKey(payload.key);
            setEditingInstanceKey(null);
            setQrConnecting(false);
            setQrPayload(null);
            setConfigOpen(false);
            setManualTestOk(false);
            autoQrStartedRef.current = null;
            message.success(ltRef.current('扫码绑定成功', 'QR bind succeeded'));
            refresh();
          } else if (payload.ok === false) {
            setQrConnecting(false);
            setQrPayload(null);
            message.error(payload.error || ltRef.current('扫码绑定失败', 'QR bind failed'));
          }
        }
        if (msg.type === 'session.list') {
          setSessions((msg.payload as { sessions?: SessionInfo[] }).sessions || []);
        }
        if (msg.type === 'session.updated') {
          const next = (msg.payload as { session?: SessionInfo }).session;
          if (!next?.id) return;
          setSessions((prev) => {
            const index = prev.findIndex((item) => item.id === next.id);
            if (index < 0) return [next, ...prev];
            const copy = prev.slice();
            copy[index] = { ...copy[index], ...next };
            return copy;
          });
        }
        if (msg.type === 'session.message') {
          const payload = msg.payload as {
            sessionId?: string;
            message?: ChannelChatMessage;
          };
          if (
            payload.sessionId &&
            payload.sessionId === selectedSessionRef.current &&
            payload.message &&
            (payload.message.role === 'user' || payload.message.role === 'assistant')
          ) {
            appendHistoryMessage(payload.message);
            if (payload.message.role === 'assistant') setStreamingText('');
          }
        }
        if (msg.type === 'session.history') {
          const payload = msg.payload as {
            sessionId?: string;
            messages?: ChannelChatMessage[];
          };
          if (payload.sessionId && payload.sessionId === selectedSessionRef.current) {
            setHistoryMessages(
              (payload.messages || []).filter((m) => m.role === 'user' || m.role === 'assistant'),
            );
            setStreamingText('');
            setHistoryLoading(false);
          }
        }
        if (msg.type === 'agent.event') {
          const payload = msg.payload as {
            sessionId?: string;
            event?: {
              type?: string;
              content?: string;
              finalResponse?: string;
              message?: { role?: string; content?: string };
            };
          };
          if (!payload.sessionId || payload.sessionId !== selectedSessionRef.current) return;
          const event = payload.event;
          if (!event?.type) return;
          if (event.type === 'text' && event.content) {
            setStreamingText((prev) => prev + event.content);
            return;
          }
          if (event.type === 'done') {
            const content = (event.finalResponse || '').trim();
            if (content) {
              appendHistoryMessage({
                role: 'assistant',
                content,
                timestamp: Date.now(),
              });
            }
            setStreamingText('');
            return;
          }
          if (event.type === 'error') {
            setStreamingText('');
          }
        }
      }),
    [activeType, addHandler, appendHistoryMessage, refresh, send],
  );

  useEffect(() => {
    if (connected) refresh();
  }, [connected, refresh]);

  useEffect(() => {
    const sync = () => {
      const next = channelTypeFromLocation();
      setActiveType((prev) => {
        if (prev === next) return prev;
        setConfigOpen(false);
        setSelectedInstanceKey(null);
        setEditingInstanceKey(null);
        setSelectedSessionId(null);
        selectedSessionRef.current = null;
        setHistoryMessages([]);
        setStreamingText('');
        setQrPayload(null);
        setQrKey(null);
        setQrConnecting(false);
        setTesting(false);
        setManualTestOk(false);
        pendingWatchRef.current = null;
        autoQrStartedRef.current = null;
        send('channel.qr.cancel', {});
        return next;
      });
    };
    window.addEventListener('hashchange', sync);
    return () => window.removeEventListener('hashchange', sync);
  }, [send]);

  const adapterByType = useMemo(() => {
    const map = new Map<string, ChannelAdapterInfo>();
    for (const item of adapters) map.set(item.type, item);
    return map;
  }, [adapters]);

  const cards = useMemo(() => {
    const known = new Set(CHANNEL_CATALOG.map((item) => item.type));
    const extras = adapters
      .filter((item) => !known.has(item.type))
      .map((item) => ({
        type: item.type,
        label: item.label || item.type,
        labelEn: item.label || item.type,
        summary: item.description || '已安装适配器',
        summaryEn: item.description || 'Installed adapter',
        tone: 'slate' as const,
        qrSupported: false,
        bindMode: 'token' as const,
      }));
    return [...CHANNEL_CATALOG, ...extras].map((meta) => {
      const adapter = adapterByType.get(meta.type);
      const typedConnections = connections.filter((item) => item.config.type === meta.type);
      const online = typedConnections.filter((item) => item.status === 'connected').length;
      const errors = typedConnections.filter((item) => item.status === 'error').length;
      return {
        ...meta,
        label: lt(meta.label, meta.labelEn),
        summary: lt(meta.summary, meta.summaryEn),
        ready: Boolean(adapter),
        setupHint: adapter?.setupHint,
        connectionCount: typedConnections.length,
        online,
        errors,
        primary: typedConnections[0] || null,
      };
    });
  }, [adapters, adapterByType, connections, lt]);

  const activeCard = cards.find((item) => item.type === activeType) || cards[0]!;
  const activeInstances = useMemo(
    () =>
      connections.filter(
        (item) =>
          item.config.type === activeType && !item.config.name.startsWith('__pending_'),
      ),
    [connections, activeType],
  );
  const selectedInstance =
    activeInstances.find((item) => keyOf(item) === selectedInstanceKey) ||
    activeInstances[0] ||
    null;
  const selectedInstanceResolvedKey = selectedInstance ? keyOf(selectedInstance) : null;

  useEffect(() => {
    if (!selectedInstanceResolvedKey) {
      if (selectedInstanceKey) setSelectedInstanceKey(null);
      return;
    }
    if (selectedInstanceKey !== selectedInstanceResolvedKey) {
      setSelectedInstanceKey(selectedInstanceResolvedKey);
    }
  }, [selectedInstanceKey, selectedInstanceResolvedKey]);

  const instanceSessions = useMemo(() => {
    if (!selectedInstance) return [];
    return sessions
      .filter((session) => isInstanceSession(session, activeType, selectedInstance))
      .sort(
        (a, b) =>
          (b.lastMessageAt || b.updatedAt || b.createdAt) -
          (a.lastMessageAt || a.updatedAt || a.createdAt),
      );
  }, [sessions, activeType, selectedInstance]);

  const selectedSession = instanceSessions.find((s) => s.id === selectedSessionId) || null;

  const beginDraftQr = useCallback(() => {
    if (!activeCard?.qrSupported || !connected || !activeCard.ready) return;
    keysBeforeDraftRef.current = new Set(connections.map((item) => keyOf(item)));
    setQrConnecting(true);
    setQrPayload(null);
    send('channel.qr.begin', {
      type: activeType,
      tenantId: form.getFieldValue('tenantId')?.trim() || 'default-tenant',
    });
  }, [activeCard, activeType, connected, connections, form, send]);

  const beginDraftQrRef = useRef(beginDraftQr);
  beginDraftQrRef.current = beginDraftQr;

  useEffect(() => {
    if (!configOpen) return;
    if (!connected || !activeCard?.qrSupported || !activeCard.ready) return;
    // 仅新建时自动出码；编辑已有实例不打扰
    if (editingInstanceKey) return;
    const marker = `modal:${activeType}:qr`;
    if (autoQrStartedRef.current === marker) return;
    autoQrStartedRef.current = marker;
    beginDraftQrRef.current();
  }, [configOpen, connected, activeCard, activeType, editingInstanceKey]);

  const selectInstance = (key: string) => {
    if (key === selectedInstanceKey) return;
    setSelectedInstanceKey(key);
    setSelectedSessionId(null);
    selectedSessionRef.current = null;
    setHistoryMessages([]);
    setStreamingText('');
  };

  const closeConfig = () => {
    if (submitting) return;
    send('channel.qr.cancel', {});
    pendingWatchRef.current = null;
    setConfigOpen(false);
    setEditingInstanceKey(null);
    setQrConnecting(false);
    setQrPayload(null);
    setQrKey(null);
    setTesting(false);
    setManualTestOk(false);
    autoQrStartedRef.current = null;
  };

  const fillFormFromInstance = (instance: ChannelConnection | null, fallbackName: string) => {
    const cfg = instance?.config;
    const storedExtra = cfg?.extra || {};
    const type = cfg?.type || activeType;
    form.setFieldsValue({
      tenantId: cfg?.tenantId || 'default-tenant',
      mentionOnly: Boolean(cfg?.mentionOnly),
      proactiveEnabled: storedExtra.proactiveEnabled !== 'false',
      name: cfg?.name || fallbackName,
      token: undefined,
      appId: storedExtra.appId,
      accountId: storedExtra.accountId,
      sandbox: storedExtra.sandbox === 'true',
      verificationToken: storedExtra.verificationToken,
      webhookPort:
        storedExtra.webhookPort ||
        (type === 'feishu'
          ? '18911'
          : type === 'dingtalk'
            ? '18912'
            : type === 'whatsapp'
              ? '18913'
              : type === 'line'
                ? '18914'
                : undefined),
      phoneNumberId: storedExtra.phoneNumberId,
      verifyToken: storedExtra.verifyToken,
      appSecret: undefined,
      channelSecret: storedExtra.channelSecret,
      channelId: storedExtra.channelId || (type === 'line' ? cfg?.name : undefined),
      proxyUrl: storedExtra.proxyUrl,
      proactiveChannelId: storedExtra.proactiveChannelId,
      allowlist: cfg?.allowlist?.length ? cfg.allowlist.join(', ') : undefined,
    });
  };

  const openCreateInstance = () => {
    setEditingInstanceKey(null);
    fillFormFromInstance(null, '');
    autoQrStartedRef.current = null;
    setQrPayload(null);
    setQrConnecting(false);
    setQrKey(null);
    setManualTestOk(false);
    setTesting(false);
    pendingWatchRef.current = null;
    setConfigOpen(true);
  };

  const openEditInstance = (instance: ChannelConnection) => {
    setEditingInstanceKey(keyOf(instance));
    fillFormFromInstance(instance, instance.config.name);
    autoQrStartedRef.current = null;
    setQrPayload(null);
    setQrConnecting(false);
    setQrKey(null);
    setManualTestOk(instance.status === 'connected');
    setTesting(false);
    pendingWatchRef.current = null;
    setConfigOpen(true);
  };

  const confirmRemoveInstance = (instance: ChannelConnection) => {
    Modal.confirm({
      title: lt('删除实例「{0}」？', 'Delete instance "{0}"?', instance.config.name),
      content: lt(
        '会断开连接并移除该配置，对话记录仍保留在会话里。',
        'Disconnects and removes this config. Conversation history stays in sessions.',
      ),
      okText: lt('删除', 'Delete'),
      okType: 'danger',
      cancelText: lt('取消', 'Cancel'),
      centered: true,
      onOk: () => {
        const key = keyOf(instance);
        send('channel.connection.remove', { key });
        if (selectedInstanceKey === key) {
          setSelectedInstanceKey(null);
          setSelectedSessionId(null);
          setHistoryMessages([]);
        }
        if (editingInstanceKey === key) {
          pendingWatchRef.current = null;
          setConfigOpen(false);
          setEditingInstanceKey(null);
          setQrConnecting(false);
          setQrPayload(null);
          setQrKey(null);
          setTesting(false);
          setManualTestOk(false);
          autoQrStartedRef.current = null;
        }
      },
    });
  };

  const editingInstance = useMemo(
    () =>
      editingInstanceKey
        ? connections.find((item) => keyOf(item) === editingInstanceKey)
        : undefined,
    [connections, editingInstanceKey],
  );

  const connectEditingInstance = () => {
    if (!editingInstance) return;
    const key = keyOf(editingInstance);
    if (activeCard.qrSupported && !editingInstance.config.token) {
      setQrConnecting(true);
      setQrPayload(null);
      send('channel.connection.connect', { key });
      return;
    }
    pendingWatchRef.current = { key, kind: 'reconnect' };
    send('channel.connection.connect', { key });
  };

  const disconnectEditingInstance = () => {
    if (!editingInstanceKey) return;
    send('channel.connection.disconnect', { key: editingInstanceKey });
  };

  const buildManualConfig = async () => {
    const values = await form.validateFields();
    const allowlist = values.allowlist
      ?.split(/[\n,]/)
      .map((item) => item.trim())
      .filter(Boolean);
    if (activeType === 'wechat') {
      if (!values.accountId?.trim()) {
        throw new Error('请填写 Bot ID');
      }
      if (!editingInstanceKey && !values.token?.trim()) {
        throw new Error('请填写 Bot Token');
      }
    } else if (!editingInstanceKey && !values.token?.trim()) {
      if (activeType === 'dingtalk') {
        throw new Error(lt('请填写加签 Secret', 'Please enter sign secret'));
      }
      if (activeType === 'qq' || activeType === 'feishu') {
        throw new Error('请填写 App Secret');
      }
      if (activeType === 'whatsapp' || activeType === 'line') {
        throw new Error('请填写 Access Token');
      }
      if (activeType === 'telegram' || activeType === 'discord') {
        throw new Error('请填写 Bot Token');
      }
    }
    if (activeType === 'whatsapp') {
      if (!values.phoneNumberId?.trim()) {
        throw new Error('请填写 Phone Number ID');
      }
      if (!values.verifyToken?.trim()) {
        throw new Error('请填写 Verify Token');
      }
      if (!editingInstanceKey && !values.appSecret?.trim()) {
        throw new Error('请填写 App Secret');
      }
    }

    const name = resolveConnectionName(activeType, values);
    const extra = Object.fromEntries(
      Object.entries({
        proactiveChannelId: values.proactiveChannelId,
        proactiveEnabled: values.proactiveEnabled === false ? 'false' : 'true',
        appId: values.appId?.trim(),
        accountId: values.accountId?.trim() || (activeType === 'wechat' ? name : undefined),
        sandbox: values.sandbox ? 'true' : undefined,
        verificationToken: values.verificationToken?.trim(),
        webhookPort: values.webhookPort?.trim(),
        phoneNumberId: values.phoneNumberId?.trim(),
        verifyToken: values.verifyToken?.trim(),
        appSecret: values.appSecret?.trim(),
        channelSecret: values.channelSecret?.trim(),
        channelId: values.channelId?.trim() || (activeType === 'line' ? name : undefined),
        proxyUrl: values.proxyUrl?.trim(),
      }).filter(([, value]) => Boolean(value)),
    );

    const key = `${activeType}:${name}`;
    return {
      key,
      previousKey:
        editingInstanceKey && editingInstanceKey !== key ? editingInstanceKey : undefined,
      config: {
        type: activeType,
        name,
        tenantId: values.tenantId.trim(),
        connectionId: key,
        token: values.token?.trim() || undefined,
        mentionOnly: Boolean(values.mentionOnly),
        allowlist,
        extra,
        enabled: true,
      },
    };
  };

  const testManualConnection = async () => {
    try {
      send('channel.qr.cancel', {});
      const { key, config } = await buildManualConfig();
      setTesting(true);
      setManualTestOk(false);
      pendingWatchRef.current = { key, kind: 'test' };
      send('channel.connection.upsert', { config, connectAfter: true });
    } catch (err) {
      setTesting(false);
      message.warning(err instanceof Error ? err.message : lt('请先完善必填项', 'Fill required fields first'));
    }
  };

  const submitManualConnection = async () => {
    if (!manualTestOk) {
      message.warning(lt('请先点击「测试连接」，通过后再保存', 'Test connection successfully before saving'));
      return;
    }
    try {
      send('channel.qr.cancel', {});
      const { key, previousKey, config } = await buildManualConfig();
      setSubmitting(true);
      pendingWatchRef.current = { key, kind: 'save', previousKey };
      send('channel.connection.upsert', { config, connectAfter: true });
      setQrConnecting(false);
      setQrPayload(null);
      setQrKey(null);
      autoQrStartedRef.current = null;
    } catch (err) {
      setSubmitting(false);
      message.warning(err instanceof Error ? err.message : lt('请先完善必填项', 'Fill required fields first'));
    }
  };

  const openSessionHistory = useCallback(
    (sessionId: string) => {
      selectedSessionRef.current = sessionId;
      setSelectedSessionId(sessionId);
      setHistoryMessages([]);
      setStreamingText('');
      setHistoryLoading(true);
      send('session.history', { sessionId });
    },
    [send],
  );

  useEffect(() => {
    if (selectedSessionId) return;
    const first = instanceSessions[0];
    if (!first) return;
    openSessionHistory(first.id);
  }, [selectedInstanceResolvedKey, instanceSessions, selectedSessionId, openSessionHistory]);

  const openInChat = (session: SessionInfo) => {
    initSession(session.id);
    addSession(session);
    setActiveSession(session.id);
    send('session.restore', { sessionId: session.id });
    navigateToPage('chat');
  };

  return (
    <AdminPage className="xy-ch-page">
      <div className="xy-ch-shell is-navless">
        <section className={`xy-ch-stage tone-${activeCard.tone}`}>
          <header className="xy-ch-toolbar">
            <div className="xy-ch-toolbar-left">
              <Icon icon={channelBrandIcon(activeCard.type)} width={22} height={22} className="xy-ch-toolbar-icon" />
              <h2>{activeCard.label}</h2>
              <span className={`xy-ch-status${activeCard.ready ? ' is-on' : ''}`}>
                {activeCard.ready ? lt('就绪', 'Ready') : lt('不可用', 'Unavailable')}
              </span>
              <span className="xy-ch-status">
                {lt('{0} 个实例', '{0} instances', activeInstances.length)}
                {activeInstances.filter((i) => i.status === 'connected').length
                  ? ` · ${lt('{0} 在线', '{0} online', activeInstances.filter((i) => i.status === 'connected').length)}`
                  : ''}
              </span>
            </div>
            <div className="xy-ch-toolbar-right">
              {!activeCard.ready ? (
                <Button
                  type="text"
                  size="small"
                  className="xy-ch-tool-btn"
                  icon={<ReloadOutlined />}
                  onClick={refresh}
                >
                  {lt('刷新适配器', 'Refresh adapters')}
                </Button>
              ) : (
                <Button
                  type="text"
                  size="small"
                  className="xy-ch-tool-btn"
                  icon={<PlusOutlined />}
                  onClick={openCreateInstance}
                >
                  {lt('新建实例', 'New instance')}
                </Button>
              )}
              <Button
                type="text"
                size="small"
                className="xy-ch-tool-btn"
                icon={<ReloadOutlined />}
                onClick={refresh}
              >
                {lt('刷新', 'Refresh')}
              </Button>
            </div>
          </header>

          {!activeCard.ready ? (
            <div className="xy-ch-empty-panel">
              <ApiOutlined />
              <h3>{lt('{0} 适配器暂不可用', '{0} adapter unavailable', activeCard.label)}</h3>
              <p>
                {lt(
                  '引擎未返回该频道适配器，请刷新或检查本地服务是否已启动。',
                  'The engine did not report this channel adapter. Refresh or check that the local service is running.',
                )}
              </p>
            </div>
          ) : (
            <div
              className={[
                'xy-ch-workspace',
                'is-tri',
                instancesCollapsed ? 'is-inst-collapsed' : '',
                sessionsCollapsed ? 'is-sess-collapsed' : '',
              ]
                .filter(Boolean)
                .join(' ')}
            >
              <aside className={`xy-ch-col xy-ch-instances${instancesCollapsed ? ' is-collapsed' : ''}`}>
                <div className="xy-ch-col-head">
                  <h3>{lt('连接实例', 'Instances')}</h3>
                  <button
                    type="button"
                    className="xy-ch-icon-btn"
                    title={instancesCollapsed ? lt('展开', 'Expand') : lt('折叠', 'Collapse')}
                    onClick={() => setInstancesCollapsed((v) => !v)}
                  >
                    {instancesCollapsed ? <MenuUnfoldOutlined /> : <MenuFoldOutlined />}
                  </button>
                </div>
                {!instancesCollapsed &&
                  (activeInstances.length === 0 ? (
                    <div className="xy-ch-talk-empty">
                      {lt('还没有实例，点右上角新建', 'No instances yet — create one above')}
                    </div>
                  ) : (
                    <ul className="xy-ch-instance-list">
                      {activeInstances.map((instance) => {
                        const key = keyOf(instance);
                        const active = key === selectedInstanceResolvedKey;
                        return (
                          <li key={key} className={active ? 'is-active' : undefined}>
                            <button
                              type="button"
                              className="xy-ch-instance-main"
                              onClick={() => selectInstance(key)}
                            >
                              <i className={`xy-ch-instance-dot is-${instance.status}`} aria-hidden />
                              <span className="xy-ch-instance-copy">
                                <strong>
                                  {instance.config.name.startsWith('__pending_')
                                    ? lt('扫码绑定中…', 'QR binding…')
                                    : instance.config.name}
                                </strong>
                                <em>{statusLabel(instance.status, lt)}</em>
                              </span>
                            </button>
                            <div className="xy-ch-instance-actions">
                              <button
                                type="button"
                                className="xy-ch-icon-btn"
                                title={lt('编辑配置', 'Edit')}
                                onClick={() => openEditInstance(instance)}
                              >
                                <SettingOutlined />
                              </button>
                            </div>
                          </li>
                        );
                      })}
                    </ul>
                  ))}
              </aside>

              <aside className={`xy-ch-col xy-ch-sessions${sessionsCollapsed ? ' is-collapsed' : ''}`}>
                <div className="xy-ch-col-head">
                  <h3>{lt('会话', 'Sessions')}</h3>
                  <button
                    type="button"
                    className="xy-ch-icon-btn"
                    title={sessionsCollapsed ? lt('展开', 'Expand') : lt('折叠', 'Collapse')}
                    onClick={() => setSessionsCollapsed((v) => !v)}
                  >
                    {sessionsCollapsed ? <MenuUnfoldOutlined /> : <MenuFoldOutlined />}
                  </button>
                </div>
                {!sessionsCollapsed &&
                  (!selectedInstance ? (
                    <div className="xy-ch-talk-empty">
                      {lt('先选一个连接实例', 'Select an instance first')}
                    </div>
                  ) : instanceSessions.length === 0 ? (
                    <div className="xy-ch-talk-empty">
                      {lt('该实例还没有对话', 'No conversations yet')}
                    </div>
                  ) : (
                    <ul className="xy-ch-session-list">
                      {instanceSessions.map((session) => {
                        const when = new Date(
                          session.lastMessageAt || session.updatedAt || session.createdAt,
                        );
                        return (
                          <li key={session.id}>
                            <button
                              type="button"
                              className={selectedSessionId === session.id ? 'is-active' : undefined}
                              onClick={() => openSessionHistory(session.id)}
                            >
                              <strong>{displaySessionName(session.name)}</strong>
                              <span>
                                {lt('{0} 条', '{0} msgs', session.messageCount)} ·{' '}
                                {when.toLocaleString(locale, {
                                  year: 'numeric',
                                  month: 'numeric',
                                  day: 'numeric',
                                  hour: '2-digit',
                                  minute: '2-digit',
                                  second: '2-digit',
                                  hour12: false,
                                })}
                              </span>
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  ))}
              </aside>

              <div className="xy-ch-talk-main">
                <div className="xy-ch-talk-main-head">
                  <h3>
                    {selectedSession
                      ? displaySessionName(selectedSession.name)
                      : lt('选择会话', 'Select a session')}
                  </h3>
                  {selectedSession ? (
                    <Button type="link" size="small" onClick={() => openInChat(selectedSession)}>
                      {lt('在聊天页打开', 'Open in chat')}
                    </Button>
                  ) : null}
                </div>
                <ChannelTranscript
                  messages={historyMessages}
                  loading={historyLoading}
                  typing={Boolean(selectedSession?.isProcessing) || streamingText.length > 0}
                  streamingText={streamingText}
                  emptyHint={
                    selectedSession
                      ? lt('这个会话还没有消息', 'No messages in this session')
                      : selectedInstance
                        ? lt('从中间选一个会话查看', 'Pick a session to view')
                        : lt('先新建或选择一个连接实例', 'Create or select an instance first')
                  }
                />
              </div>
            </div>
          )}
        </section>
      </div>

      <Modal
        title={null}
        open={configOpen}
        onCancel={closeConfig}
        width={activeCard.qrSupported ? 860 : 780}
        destroyOnHidden
        centered
        className="xy-ch-config-modal"
        rootClassName={`xy-ch-config-modal-root tone-${activeCard.tone}`}
        closable
        footer={
          <div className="xy-ch-sheet-footer">
            {editingInstance ? (
              <Button
                danger
                icon={<DeleteOutlined />}
                disabled={submitting || testing}
                onClick={() => confirmRemoveInstance(editingInstance)}
              >
                {lt('删除', 'Delete')}
              </Button>
            ) : null}
            <span className="xy-ch-sheet-footer-spacer" />
            <Button onClick={closeConfig} disabled={submitting}>
              {lt('取消', 'Cancel')}
            </Button>
            {editingInstance ? (
              editingInstance.status === 'connected' ? (
                <Button
                  icon={<DisconnectOutlined />}
                  disabled={submitting || testing}
                  onClick={disconnectEditingInstance}
                >
                  {lt('断开', 'Disconnect')}
                </Button>
              ) : (
                <Button
                  icon={<LinkOutlined />}
                  disabled={submitting || testing}
                  onClick={connectEditingInstance}
                >
                  {lt('连接', 'Connect')}
                </Button>
              )
            ) : null}
            {activeCard.qrSupported ? (
              <Button
                icon={<QrcodeOutlined />}
                loading={qrConnecting && !qrPayload}
                disabled={testing || submitting}
                onClick={() => {
                  autoQrStartedRef.current = null;
                  beginDraftQr();
                }}
              >
                {lt('刷新二维码', 'Refresh QR')}
              </Button>
            ) : null}
            <Button
              icon={<ApiOutlined />}
              loading={testing}
              disabled={submitting}
              onClick={() => void testManualConnection()}
            >
              {lt('测试连接', 'Test connection')}
            </Button>
            <Button
              type="primary"
              className="xy-ch-save-btn"
              loading={submitting}
              disabled={!manualTestOk || testing}
              onClick={() => void submitManualConnection().catch(() => setSubmitting(false))}
            >
              {lt('保存', 'Save')}
            </Button>
          </div>
        }
      >
        <div className="xy-ch-sheet">
          <header className="xy-ch-sheet-head">
            <div className="xy-ch-sheet-brand">
              <span className="xy-ch-sheet-mark" aria-hidden>
                <Icon icon={channelBrandIcon(activeCard.type)} width={28} height={28} />
              </span>
              <div className="xy-ch-sheet-titles">
                <h2>
                  {editingInstanceKey
                    ? lt('编辑 {0} 实例', 'Edit {0} instance', activeCard.label)
                    : lt('新建 {0} 实例', 'New {0} instance', activeCard.label)}
                </h2>
                <p>{activeCard.summary}</p>
              </div>
            </div>
            {!activeCard.qrSupported ? (
              <span className="xy-ch-sheet-mode-tag">
                {activeCard.bindMode === 'webhook' ? 'Webhook' : 'Token'}
              </span>
            ) : null}
          </header>

          {activeCard.qrSupported ? (
            <div className="xy-ch-sheet-split">
              <div className="xy-ch-sheet-qr">
                <ChannelQrBoard
                  payload={qrPayload}
                  loading={qrConnecting && !qrPayload}
                  size={132}
                  statusText={
                    qrPayload
                      ? lt('扫码后在手机上确认即可绑定', 'Confirm on your phone after scanning')
                      : qrConnecting
                        ? lt('正在申请二维码…', 'Requesting QR…')
                        : lt('准备出码', 'Ready')
                  }
                />
              </div>
              <div className="xy-ch-sheet-side is-form">{renderManualForm()}</div>
            </div>
          ) : (
            <div className="xy-ch-sheet-body">{renderManualForm()}</div>
          )}
        </div>
      </Modal>

    </AdminPage>
  );

  function field(
    name: keyof ChannelFormValues,
    label: string,
    control: React.ReactNode,
    opts?: { required?: boolean; message?: string; span2?: boolean },
  ) {
    return (
      <Form.Item
        name={name}
        label={label}
        rules={opts?.required ? [{ required: true, message: opts.message || lt('请填写 {0}', 'Please enter {0}', label) }] : undefined}
        className={opts?.span2 ? 'is-span-2' : undefined}
      >
        {control}
      </Form.Item>
    );
  }

  function renderManualForm() {
    const secretRequired = !editingInstanceKey;
    const credentialFields = (() => {
      if (activeType === 'qq') {
        return (
          <>
            {field('appId', 'App ID', <Input placeholder={lt('从 q.qq.com 获取', 'From q.qq.com')} autoComplete="off" />, {
              required: true,
            })}
            {field(
              'token',
              'App Secret',
              <Input.Password placeholder="AppSecret" autoComplete="new-password" />,
              { required: secretRequired },
            )}
          </>
        );
      }
      if (activeType === 'wechat') {
        return (
          <>
            {field(
              'accountId',
              'Bot ID',
              <Input
                placeholder={lt('扫码后自动填充，或粘贴 xxx@im.bot', 'Auto-filled after QR, or paste xxx@im.bot')}
                autoComplete="off"
              />,
              { required: !activeCard.qrSupported },
            )}
            {field(
              'token',
              'Bot Token',
              <Input.Password
                placeholder={lt('可选，已有可粘贴', 'Optional if already bound')}
                autoComplete="new-password"
              />,
            )}
          </>
        );
      }
      if (activeType === 'telegram') {
        return (
          <>
            {field(
              'token',
              'Bot Token',
              <Input.Password
                placeholder={lt('从 BotFather 获取', 'From BotFather')}
                autoComplete="new-password"
              />,
              { required: secretRequired, span2: true },
            )}
            {field(
              'proxyUrl',
              lt('HTTP 代理', 'HTTP Proxy'),
              <Input
                placeholder={lt('如 http://127.0.0.1:7890，留空则读 HTTPS_PROXY', 'e.g. http://127.0.0.1:7890; empty uses HTTPS_PROXY')}
                autoComplete="off"
              />,
              { span2: true },
            )}
          </>
        );
      }
      if (activeType === 'feishu') {
        return (
          <>
            {field('appId', 'App ID', <Input autoComplete="off" />, { required: true })}
            {field(
              'token',
              'App Secret',
              <Input.Password autoComplete="new-password" />,
              { required: secretRequired },
            )}
            {field('verificationToken', 'Verification', <Input autoComplete="off" />)}
            {field(
              'webhookPort',
              lt('Webhook 端口', 'Webhook port'),
              <Input placeholder="18911" autoComplete="off" />,
            )}
          </>
        );
      }
      if (activeType === 'dingtalk') {
        return (
          <>
            {field(
              'token',
              lt('加签 Secret', 'Sign secret'),
              <Input.Password autoComplete="new-password" />,
              { required: secretRequired },
            )}
            {field(
              'webhookPort',
              lt('Webhook 端口', 'Webhook port'),
              <Input placeholder="18912" autoComplete="off" />,
            )}
          </>
        );
      }
      if (activeType === 'whatsapp') {
        return (
          <>
            {field(
              'token',
              'Access Token',
              <Input.Password autoComplete="new-password" />,
              { required: secretRequired },
            )}
            {field('phoneNumberId', 'Phone Number ID', <Input autoComplete="off" />, {
              required: true,
            })}
            {field('verifyToken', 'Verify Token', <Input autoComplete="off" />, {
              required: true,
            })}
            {field(
              'appSecret',
              'App Secret',
              <Input.Password autoComplete="new-password" />,
              { required: secretRequired },
            )}
            {field(
              'webhookPort',
              lt('Webhook 端口', 'Webhook port'),
              <Input placeholder="18913" autoComplete="off" />,
            )}
          </>
        );
      }
      if (activeType === 'line') {
        return (
          <>
            {field(
              'channelId',
              'Channel ID',
              <Input placeholder={lt('LINE Developers 控制台', 'LINE Developers console')} autoComplete="off" />,
              { required: true },
            )}
            {field(
              'token',
              'Access Token',
              <Input.Password autoComplete="new-password" />,
              { required: secretRequired },
            )}
            {field(
              'channelSecret',
              'Channel Secret',
              <Input.Password autoComplete="new-password" />,
              { required: secretRequired },
            )}
            {field(
              'webhookPort',
              lt('Webhook 端口', 'Webhook port'),
              <Input placeholder="18914" autoComplete="off" />,
              { span2: true },
            )}
          </>
        );
      }
      if (activeType === 'discord') {
        return field(
          'token',
          'Bot Token',
          <Input.Password
            placeholder={lt('Discord Bot Token', 'Discord Bot Token')}
            autoComplete="new-password"
          />,
          { required: secretRequired, span2: true },
        );
      }
      return field(
        'token',
        'Bot Token',
        <Input.Password autoComplete="new-password" />,
        { required: secretRequired, span2: true },
      );
    })();

    return (
      <Form
        form={form}
        layout="horizontal"
        className="xy-channel-form"
        requiredMark={false}
        colon={false}
        onValuesChange={(_, all) => {
          if (manualTestOk) setManualTestOk(false);
          if (!usesOfficialIdentity(activeType)) return;
          try {
            const next = resolveConnectionName(activeType, all);
            if (all.name !== next) form.setFieldValue('name', next);
          } catch {
            /* 凭证未填全 */
          }
        }}
      >
        <div className="xy-ch-sheet-section">
          <h3>{lt('身份与凭证', 'Identity & credentials')}</h3>
          <div className="xy-ch-field-grid">
            {usesOfficialIdentity(activeType)
              ? null
              : field(
                  'name',
                  lt('显示名称', 'Display name'),
                  <Input placeholder={activeCard.label} autoComplete="off" />,
                  { required: true },
                )}
            {field(
              'tenantId',
              lt('租户标识', 'Tenant ID'),
              <Input placeholder="default-tenant" autoComplete="off" />,
              { required: true },
            )}
            {credentialFields}
            {field(
              'proactiveChannelId',
              lt('主动目标', 'Proactive target'),
              <Input placeholder={lt('可选', 'Optional')} autoComplete="off" />,
            )}
            {field(
              'allowlist',
              lt('用户白名单', 'Allowlist'),
              <Input placeholder={lt('用户 ID，逗号分隔', 'User IDs, comma-separated')} autoComplete="off" />,
            )}
          </div>
        </div>

        <div className="xy-ch-sheet-section">
          <h3>{lt('行为', 'Behavior')}</h3>
          <div className="xy-ch-pref-bar">
            {activeType === 'qq' ? (
              <label className="xy-ch-pref">
                <span>
                  <em>{lt('沙箱', 'Sandbox')}</em>
                  <small>{lt('沙箱接口', 'Sandbox API')}</small>
                </span>
                <Form.Item name="sandbox" valuePropName="checked" noStyle>
                  <Switch size="small" />
                </Form.Item>
              </label>
            ) : null}
            <label className="xy-ch-pref">
              <span>
                <em>{lt('主动消息', 'Proactive')}</em>
                <small>{lt('允许推送', 'Allow push')}</small>
              </span>
              <Form.Item name="proactiveEnabled" valuePropName="checked" noStyle>
                <Switch size="small" />
              </Form.Item>
            </label>
            <label className="xy-ch-pref">
              <span>
                <em>{lt('仅提及', 'Mention only')}</em>
                <small>{lt('需 @机器人', 'Requires @bot')}</small>
              </span>
              <Form.Item name="mentionOnly" valuePropName="checked" noStyle>
                <Switch size="small" />
              </Form.Item>
            </label>
          </div>
        </div>
      </Form>
    );
  }
}
