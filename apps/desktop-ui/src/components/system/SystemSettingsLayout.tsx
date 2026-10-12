/**
 * SystemSettingsLayout — settings content shell (nav lives in AppSidebar).
 */

import React, { useEffect, useState } from 'react';
import { Button, Typography, message } from 'antd';
import { ReloadOutlined, SaveOutlined } from '@ant-design/icons';

import { useWs } from '../../App';
import { useI18n, useLocaleText } from '../../i18n';
import { navigateToPage } from '../../navigation/pageRegistry';
import { useConfigStore, type AppConfig } from '../../stores/configStore';
import ConfigSectionView from './ConfigPanel';
import SandboxPanel from './SandboxPanel';
import SystemStatusPanel from './SystemStatusPanel';
import AgentPanel from '../tasks/AgentPanel';
import {
  isEditableSettingsSection,
  settingsNavMeta,
  settingsSectionFromLocation,
  type SettingsSection,
} from './settingsSections';

const { Text } = Typography;

/** 被谁调用：系统路由。做什么：设置分区内容壳。 */
export default function SystemSettingsLayout({
  fallback,
}: {
  fallback: React.ReactNode;
}) {
  const { send } = useWs();
  const { t } = useI18n();
  const lt = useLocaleText();
  const storeConfig = useConfigStore((s) => s.config);
  const capabilities = useConfigStore((s) => s.capabilities);
  const [section, setSection] = useState<SettingsSection>(settingsSectionFromLocation);
  const [draft, setDraft] = useState<AppConfig | null>(storeConfig);

  useEffect(() => {
    setDraft(storeConfig);
  }, [storeConfig]);

  useEffect(() => {
    const sync = () => setSection(settingsSectionFromLocation());
    window.addEventListener('hashchange', sync);
    return () => window.removeEventListener('hashchange', sync);
  }, []);

  const activeMeta = settingsNavMeta(section);
  const editable = isEditableSettingsSection(section);

  const save = () => {
    if (!draft) return;
    send('config.update', { config: draft });
    message.success(lt('配置已发送保存', 'Config sent for saving'));
  };

  const refresh = () => {
    send('config.get', {});
    if (section === 'status') {
      send('system.status', {});
      send('gateway.logs', { limit: 200 });
    }
    if (section === 'tasks' || section === 'team') {
      send('agent.list', {});
    }
  };

  return (
    <div className="xy-settings-shell is-navless">
      <div className="xy-settings-main">
        <header className="xy-settings-header">
          <div>
            <h2 className="xy-settings-title">{t(activeMeta.labelKey, activeMeta.fallback)}</h2>
            <Text type="secondary" className="xy-settings-desc">
              {t(activeMeta.descKey, activeMeta.descFallback)}
            </Text>
          </div>
          <Button onClick={() => navigateToPage('setup')}>
            {lt('配置向导', 'Setup wizard')}
          </Button>
        </header>

        <div className="xy-settings-body">
          <React.Suspense fallback={fallback}>
            {!draft && editable ? (
              <Text type="secondary">{lt('正在加载配置...', 'Loading config...')}</Text>
            ) : (
              <>
                {section === 'models' && draft && (
                  <ConfigSectionView
                    section="models"
                    config={draft}
                    capabilities={capabilities}
                    onChange={setDraft}
                  />
                )}
                {section === 'team' && draft && (
                  <ConfigSectionView
                    section="team"
                    config={draft}
                    capabilities={capabilities}
                    onChange={setDraft}
                  />
                )}
                {section === 'tasks' && <AgentPanel />}
                {section === 'permission' && draft && (
                  <ConfigSectionView
                    section="permission"
                    config={draft}
                    capabilities={capabilities}
                    onChange={setDraft}
                  />
                )}
                {section === 'context' && draft && (
                  <ConfigSectionView
                    section="context"
                    config={draft}
                    capabilities={capabilities}
                    onChange={setDraft}
                  />
                )}
                {section === 'storage' && draft && (
                  <ConfigSectionView
                    section="storage"
                    config={draft}
                    capabilities={capabilities}
                    onChange={setDraft}
                  />
                )}
                {section === 'sandbox' && draft && (
                  <SandboxPanel config={draft} onChange={setDraft} embedded />
                )}
                {section === 'status' && <SystemStatusPanel />}
              </>
            )}
          </React.Suspense>
        </div>

        {editable && (
          <footer className="xy-settings-footer">
            <Button icon={<ReloadOutlined />} onClick={refresh}>
              {lt('刷新', 'Refresh')}
            </Button>
            <Button type="primary" icon={<SaveOutlined />} onClick={save} disabled={!draft}>
              {lt('保存配置', 'Save config')}
            </Button>
          </footer>
        )}
      </div>
    </div>
  );
}
