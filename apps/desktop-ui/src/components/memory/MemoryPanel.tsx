/**
 * MemoryPanel - 记忆控制台
 *
 * 管理 profile 文件与长期记忆库。
 */

import React, { useEffect, useState } from 'react';

import {
  Button,
  Empty,
  Form,
  Input,
  InputNumber,
  message,
  Modal,
  Popconfirm,
  Select,
  Space,
  Spin,
  Tabs,
  Tag,
  Typography,
} from 'antd';

import {
  ClockCircleOutlined,
  DeleteOutlined,
  EditOutlined,
  FileTextOutlined,
  PlusOutlined,
  ReloadOutlined,
  SearchOutlined,
  TagOutlined,
} from '@ant-design/icons';

import { useWs } from '../../App';
import { useI18n } from '../../i18n';
import type {
  MemoryItem,
  MemoryProfileFile,
} from '../../protocol';
import { useChatStore } from '../../stores/chatStore';

import MarkdownSplitEditor from './MarkdownSplitEditor';

const { Text, Title } = Typography;

interface MemoryFormValues {
  content: string;
  category: string;
  importance: number;
  tags?: string;
}

const CATEGORY_OPTIONS = [
  'identity',
  'user_profile',
  'user_preference',
  'project_fact',
  'workflow_rule',
  'task_lesson',
  'error_pattern',
  'code_knowledge',
  'decision',
  'session',
];

const CATEGORY_COLORS: Record<string, string> = {
  identity: 'gold',
  user_profile: 'magenta',
  user_preference: 'purple',
  project_fact: 'green',
  workflow_rule: 'blue',
  task_lesson: 'cyan',
  error_pattern: 'red',
  error_experience: 'red',
  code_knowledge: 'geekblue',
  decision: 'orange',
  session: 'default',
};

const PROFILE_LABEL_KEYS: Record<MemoryProfileFile['kind'], string> = {
  identity: 'memory.profile.identity',
  user: 'memory.profile.user',
  project: 'memory.profile.project',
  agents: 'memory.profile.agents',
  workflow: 'memory.profile.workflow',
  lessons: 'memory.profile.lessons',
};

/** 被谁调用：系统/资源导航。做什么：记忆 CRUD 与 profile 编辑；经 WS memory.*。 */
export default function MemoryPanel() {
  const { send, addHandler } = useWs();
  const { t, locale } = useI18n();
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const [memories, setMemories] = useState<MemoryItem[]>([]);
  const [profileFiles, setProfileFiles] = useState<MemoryProfileFile[]>([]);
  const [loading, setLoading] = useState(false);
  const [profileLoading, setProfileLoading] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [stats, setStats] = useState<{ total: number; categories: Record<string, number> } | null>(null);
  const [editingMemory, setEditingMemory] = useState<MemoryItem | null>(null);
  const [editingProfile, setEditingProfile] = useState<MemoryProfileFile | null>(null);
  const [profileContent, setProfileContent] = useState('');
  const [modalOpen, setModalOpen] = useState(false);
  const [form] = Form.useForm<MemoryFormValues>();

  useEffect(() => addHandler((msg) => {
    if (msg.type === 'memory.list' || msg.type === 'memory.search') {
      const payload = msg.payload as {
        sessionId: string;
        memories: MemoryItem[];
        stats: { total: number; categories: Record<string, number> };
      };
      if (payload.sessionId !== activeSessionId) return;
      setMemories(payload.memories || []);
      setStats(payload.stats || null);
      setLoading(false);
    }

    if (msg.type === 'memory.profile.list') {
      const payload = msg.payload as { sessionId: string; files: MemoryProfileFile[] };
      if (payload.sessionId !== activeSessionId) return;
      setProfileFiles(payload.files || []);
      setProfileLoading(false);
    }

    if (msg.type === 'memory.profile.read') {
      const payload = msg.payload as { sessionId: string; file: MemoryProfileFile };
      if (payload.sessionId !== activeSessionId) return;
      setEditingProfile(payload.file);
      setProfileContent(payload.file.content || '');
      setProfileLoading(false);
    }

    if (msg.type === 'memory.profile.updated') {
      const payload = msg.payload as { sessionId: string; file: MemoryProfileFile; files: MemoryProfileFile[] };
      if (payload.sessionId !== activeSessionId) return;
      setProfileFiles(payload.files || []);
      setEditingProfile(payload.file);
      setProfileContent(payload.file.content || '');
      message.success(t('memory.profileSaved'));
      send('memory.list', { sessionId: activeSessionId, limit: 50 });
    }

    if (msg.type === 'memory.deleted') {
      const payload = msg.payload as { sessionId: string; id: string; deleted: boolean };
      if (payload.sessionId !== activeSessionId) return;
      if (payload.deleted) {
        setMemories((items) => items.filter((m) => m.id !== payload.id));
        message.success(t('memory.deleted'));
      } else {
        message.error(t('memory.deleteFailed'));
      }
    }

    if (msg.type === 'memory.created') {
      const payload = msg.payload as {
        sessionId: string;
        memory: MemoryItem;
        stats: { total: number; categories: Record<string, number> };
      };
      if (payload.sessionId !== activeSessionId) return;
      setMemories((items) => [payload.memory, ...items]);
      setStats(payload.stats || null);
      setModalOpen(false);
      setEditingMemory(null);
      form.resetFields();
      message.success(t('memory.created'));
    }

    if (msg.type === 'memory.updated') {
      const payload = msg.payload as {
        sessionId: string;
        id: string;
        updated: boolean;
        memory?: MemoryItem;
        stats: { total: number; categories: Record<string, number> };
      };
      if (payload.sessionId !== activeSessionId) return;
      if (payload.updated && payload.memory) {
        setMemories((items) => items.map((memory) => memory.id === payload.id ? payload.memory! : memory));
        setStats(payload.stats || null);
        setModalOpen(false);
        setEditingMemory(null);
        form.resetFields();
        message.success(t('memory.updated'));
      } else {
        message.error(t('memory.updateFailed'));
      }
    }

    if (msg.type === 'memory.flushed') {
      const payload = msg.payload as { sessionId: string };
      if (payload.sessionId !== activeSessionId) return;
      message.success(t('memory.flushed'));
    }

    if (msg.type === 'error') {
      setLoading(false);
      setProfileLoading(false);
    }
  }), [activeSessionId, addHandler, form, send, t]);

  useEffect(() => {
    if (!activeSessionId) return;
    loadMemories();
    loadProfile();
  }, [activeSessionId]);

  const ensureSession = () => {
    if (activeSessionId) return true;
    message.warning(t('memory.noSession'));
    return false;
  };

  const loadMemories = () => {
    if (!ensureSession()) return;
    setLoading(true);
    send('memory.list', { sessionId: activeSessionId, limit: 50 });
  };

  const loadProfile = () => {
    if (!ensureSession()) return;
    setProfileLoading(true);
    send('memory.profile.list', { sessionId: activeSessionId });
  };

  const searchMemories = () => {
    if (!searchQuery.trim() || !ensureSession()) return;
    setLoading(true);
    send('memory.search', { sessionId: activeSessionId, query: searchQuery, limit: 20 });
  };

  const deleteMemory = (id: string) => {
    if (!ensureSession()) return;
    send('memory.delete', { sessionId: activeSessionId, id });
  };

  const openCreateModal = () => {
    setEditingMemory(null);
    form.setFieldsValue({ category: 'session', importance: 5, content: '', tags: '' });
    setModalOpen(true);
  };

  const openEditModal = (memory: MemoryItem) => {
    setEditingMemory(memory);
    form.setFieldsValue({
      content: memory.content,
      category: memory.category,
      importance: memory.importance,
      tags: memory.tags?.join(', ') || '',
    });
    setModalOpen(true);
  };

  const submitMemory = async () => {
    if (!ensureSession()) return;
    const values = await form.validateFields();
    const tags = parseTags(values.tags);
    if (editingMemory) {
      send('memory.update', {
        sessionId: activeSessionId,
        id: editingMemory.id,
        updates: {
          content: values.content,
          category: values.category,
          importance: values.importance,
          tags,
        },
      });
    } else {
      send('memory.create', {
        sessionId: activeSessionId,
        content: values.content,
        category: values.category,
        importance: values.importance,
        tags,
      });
    }
  };

  const openProfile = (file: MemoryProfileFile) => {
    if (!ensureSession()) return;
    setProfileLoading(true);
    send('memory.profile.read', { sessionId: activeSessionId, kind: file.kind });
  };

  const saveProfile = () => {
    if (!activeSessionId || !editingProfile) return;
    send('memory.profile.update', {
      sessionId: activeSessionId,
      kind: editingProfile.kind,
      content: profileContent,
    });
  };

  return (
    <div className="xy-memory-root">
      <div className="xy-memory-main">
        <Tabs
          className="xy-memory-tabs"
          items={[
            {
              key: 'profile',
              label: t('memory.profileFiles'),
              children: (
                <div className="xy-memory-pane">
                  <div className="xy-memory-split">
                    <aside className="xy-memory-split-nav">
                      <div className="xy-memory-nav-head">
                        <span className="xy-memory-nav-title">{t('memory.profileFiles')}</span>
                        <Button
                          type="text"
                          size="small"
                          icon={<ReloadOutlined />}
                          onClick={loadProfile}
                          aria-label={t('common.refresh')}
                        />
                      </div>
                      <Spin spinning={profileLoading}>
                        <div className="xy-memory-nav-list">
                          {profileFiles.length === 0 && !profileLoading ? (
                            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description={t('memory.noData')} />
                          ) : (
                            profileFiles.map((file) => {
                              const active = editingProfile?.kind === file.kind;
                              return (
                                <button
                                  key={file.kind}
                                  type="button"
                                  className={`xy-profile-item${active ? ' is-active' : ''}`}
                                  onClick={() => openProfile(file)}
                                >
                                  <div className="xy-profile-item-icon">
                                    <FileTextOutlined />
                                  </div>
                                  <div className="xy-profile-item-body">
                                    <div className="xy-profile-item-title">
                                      {t(PROFILE_LABEL_KEYS[file.kind])}
                                    </div>
                                    <div className="xy-profile-item-path" title={file.relativePath}>
                                      {file.relativePath}
                                    </div>
                                    <Tag className="xy-profile-item-scope">
                                      {file.scope === 'global' ? t('memory.global') : t('memory.project')}
                                    </Tag>
                                  </div>
                                </button>
                              );
                            })
                          )}
                        </div>
                      </Spin>
                    </aside>

                    <section className={`xy-memory-split-editor${editingProfile ? '' : ' is-empty'}`}>
                      {editingProfile ? (
                        <>
                          <header className="xy-memory-editor-toolbar">
                            <div className="xy-memory-editor-toolbar-info">
                              <Title level={5} className="xy-memory-editor-title">
                                {t(PROFILE_LABEL_KEYS[editingProfile.kind])}
                              </Title>
                              <Text className="xy-memory-editor-path" type="secondary" ellipsis>
                                {editingProfile.relativePath}
                              </Text>
                            </div>
                            <Button type="primary" onClick={saveProfile}>
                              {t('memory.saveAndSync')}
                            </Button>
                          </header>
                          <div className="xy-memory-editor-body">
                            <MarkdownSplitEditor
                              value={profileContent}
                              onChange={setProfileContent}
                              placeholder={t('memory.markdownPlaceholder')}
                            />
                          </div>
                        </>
                      ) : (
                        <Empty
                          image={Empty.PRESENTED_IMAGE_SIMPLE}
                          description={t('memory.selectProfileFile')}
                        />
                      )}
                    </section>
                  </div>
                </div>
              ),
            },
            {
              key: 'library',
              label: `${t('memory.library')}${stats?.total ? ` · ${stats.total}` : memories.length ? ` · ${memories.length}` : ''}`,
              children: (
                <div className="xy-memory-pane">
                  <div className="xy-memory-tab-bar">
                    <Input
                      className="xy-memory-search"
                      placeholder={t('memory.searchPlaceholder')}
                      prefix={<SearchOutlined />}
                      value={searchQuery}
                      onChange={(e) => setSearchQuery(e.target.value)}
                      onPressEnter={searchMemories}
                      allowClear
                    />
                    <Button icon={<SearchOutlined />} onClick={searchMemories}>
                      {t('memory.search')}
                    </Button>
                    <Button icon={<ReloadOutlined />} onClick={loadMemories}>
                      {t('common.refresh')}
                    </Button>
                    <Button type="primary" icon={<PlusOutlined />} onClick={openCreateModal}>
                      {t('common.create')}
                    </Button>
                  </div>
                  <div className="xy-memory-list">
                    {loading ? (
                      <div className="xy-memory-list-loading">
                        <Spin tip={t('common.loading')} />
                      </div>
                    ) : memories.length === 0 ? (
                      <div className="xy-memory-list-empty">
                        <Empty description={t('memory.noData')} />
                      </div>
                    ) : (
                      memories.map((item) =>
                        renderMemoryItem(item, openEditModal, deleteMemory, t, locale),
                      )
                    )}
                  </div>
                </div>
              ),
            },
          ]}
        />
      </div>

      <Modal
        title={editingMemory ? t('memory.edit') : t('memory.create')}
        open={modalOpen}
        onOk={submitMemory}
        onCancel={() => {
          setModalOpen(false);
          setEditingMemory(null);
          form.resetFields();
        }}
        okText={editingMemory ? t('common.save') : t('common.create')}
        cancelText={t('common.cancel')}
        destroyOnHidden
      >
        <Form form={form} layout="vertical" initialValues={{ category: 'session', importance: 5 }}>
          <Form.Item
            name="content"
            label={t('memory.content')}
            rules={[{ required: true, whitespace: true, message: t('memory.contentRequired') }]}
          >
            <Input.TextArea rows={5} maxLength={4000} showCount />
          </Form.Item>
          <Form.Item name="category" label={t('memory.category')}>
            <Select options={CATEGORY_OPTIONS.map((value) => ({ value, label: value }))} />
          </Form.Item>
          <Form.Item name="importance" label={t('memory.importance')}>
            <InputNumber min={1} max={10} precision={0} style={{ width: '100%' }} />
          </Form.Item>
          <Form.Item name="tags" label={t('memory.tags')}>
            <Input placeholder={t('memory.tagsPlaceholder')} />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}

/** 被谁调用：MemoryPanel 列表。做什么：渲染单条记忆卡片（分类/标签/操作）。 */
function renderMemoryItem(
  item: MemoryItem,
  onEdit: (item: MemoryItem) => void,
  onDelete: (id: string) => void,
  t: (key: string, fallback?: string, ...args: Array<string | number>) => string,
  locale: 'zh-CN' | 'en',
) {
  return (
    <div key={item.id} className="xy-list-row">
      <div className="xy-list-row-main">
        <div className="xy-list-row-meta">
          <Tag color={CATEGORY_COLORS[item.category] || 'default'}>{item.category}</Tag>
          {item.explicit && <Tag color="gold">{t('memory.explicit')}</Tag>}
          <Text type="secondary">
            {t('memory.importance')} {item.importance}/10
          </Text>
          <Text type="secondary">
            <ClockCircleOutlined /> {new Date(item.createdAt).toLocaleString(locale)}
          </Text>
        </div>
        <div className="xy-list-row-body">{item.content}</div>
        {item.tags?.length ? (
          <Space size={4} wrap style={{ marginTop: 8 }}>
            {item.tags.map((tag) => (
              <Tag key={tag} icon={<TagOutlined />}>
                {tag}
              </Tag>
            ))}
          </Space>
        ) : null}
      </div>
      <div className="xy-list-row-actions">
        <Button type="text" icon={<EditOutlined />} size="small" onClick={() => onEdit(item)} />
        <Popconfirm
          title={t('memory.deleteConfirm')}
          onConfirm={() => onDelete(item.id)}
          okText={t('common.delete')}
          cancelText={t('common.cancel')}
        >
          <Button type="text" danger icon={<DeleteOutlined />} size="small" />
        </Popconfirm>
      </div>
    </div>
  );
}

/** 被谁调用：MemoryPanel 保存。做什么：逗号/空格切 tags。 */
function parseTags(value?: string): string[] | undefined {
  const tags = (value || '')
    .split(/[,，]/)
    .map((tag) => tag.trim())
    .filter(Boolean);
  return tags.length > 0 ? tags : undefined;
}
