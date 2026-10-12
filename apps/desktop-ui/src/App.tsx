/**
 * App.tsx - 应用根组件
 */

import React, { useEffect, useMemo, useState } from 'react';

import { ConfigProvider, App as AntdApp } from 'antd';
import enUS from 'antd/locale/en_US';
import zhCN from 'antd/locale/zh_CN';

import { useWebSocket } from './hooks/useWebSocket';
import Layout from './components/shell/Layout';
import { configGet } from './ipc/desktopIpc';
import { I18nProvider, useI18n } from './i18n';
import { ThemeProvider, useTheme } from './theme/ThemeContext';
import { buildAntdAppTheme } from './theme/antdAppTheme';
import { useConfigStore } from './stores/configStore';
import HelloXuanYuIntro from './init/HelloXuanYuIntro';
import { hasUsableInitConfig } from './init/initApi';
import { navigateToPage } from './navigation/pageRegistry';
import type { SystemStatusPayload } from './protocol';
import {
  AUTH_STORAGE_KEYS,
  persistAuthSession,
  resolveAuthBootstrap,
  stripTokenFromLocation,
} from './utils/gatewayConnection';

interface WsContextType {
  connected: boolean;
  send: (type: string, payload: unknown) => void;
  addHandler: import('./hooks/useWebSocket').AddHandler;
  mainSessionId: string;
}

export const WsContext = React.createContext<WsContextType>({
  connected: false,
  send: () => {},
  addHandler: () => () => {},
  mainSessionId: '',
});

/** 读 WS 上下文：connected / send / addHandler / mainSessionId。 */
export function useWs() {
  return React.useContext(WsContext);
}

export { useTheme } from './theme/ThemeContext';

/** 根组件：先铺满开屏动画，动画播毕即刻进入主壳（引擎就绪由 WS 层自行处理）。 */
export default function App() {
  const [introDone, setIntroDone] = useState(false);
  return (
    <I18nProvider>
      <ThemeProvider>
        {introDone ? (
          <AppShell />
        ) : (
          <div className="xy-splash">
            <HelloXuanYuIntro onDone={() => setIntroDone(true)} />
          </div>
        )}
      </ThemeProvider>
    </I18nProvider>
  );
}

/** 仅跟随服务端 config.locale，避免顶栏切换被配置强制打回。 */
function LocaleSync() {
  const { setLocale } = useI18n();
  const configLocale = useConfigStore((s) => s.config?.locale);
  // Only follow server/config changes — do NOT depend on UI locale, or header toggle
  // flashes English then gets forced back to config zh-CN.
  useEffect(() => {
    if (configLocale !== 'zh-CN' && configLocale !== 'en') return;
    setLocale(configLocale);
  }, [configLocale, setLocale]);
  return null;
}

/** 主界面：鉴权引导、WS、Ant Design 主题、Layout；监听 system.status 记网关启动时间。 */
function AppShell() {
  const bootstrap = useMemo(() => resolveAuthBootstrap(), []);
  const [gatewayUrl] = useState(() => bootstrap.gatewayUrl);
  const { locale } = useI18n();
  const { mode, antdTokens, resolved } = useTheme();
  const [didSetupCheck, setDidSetupCheck] = useState(false);

  const ws = useWebSocket({
    enabled: true,
    gatewayUrl,
    token: '',
  });

  // 首启检查：等引擎连接就绪后读取配置，配置不完整（缺 provider/model）则落到配置向导页。
  useEffect(() => {
    if (!ws.connected || didSetupCheck) return;
    let cancelled = false;
    void configGet()
      .then((config) => {
        if (cancelled) return;
        setDidSetupCheck(true);
        if (!hasUsableInitConfig(config as Record<string, unknown>)) navigateToPage('setup');
      })
      .catch(() => {
        setDidSetupCheck(true); // 读取失败不重复占用，用户仍可从菜单进入配置向导。
      });
    return () => {
      cancelled = true;
    };
  }, [ws.connected, didSetupCheck]);

  useEffect(() => {
    stripTokenFromLocation();
    persistAuthSession({
      gatewayUrl,
      token: '',
    });
  }, [gatewayUrl]);

  useEffect(() => {
    return ws.addHandler((msg) => {
      if (msg.type !== 'system.status') return;
      const startedAt = (msg.payload as SystemStatusPayload)?.runtime?.startedAt;
      if (!startedAt) return;
      window.localStorage.setItem(AUTH_STORAGE_KEYS.gatewayStartedAt, String(startedAt));
    });
  }, [ws.addHandler]);

  const antdLocale = locale === 'en' ? enUS : zhCN;
  const isDark = mode === 'dark';
  const t = resolved.tokens;

  const mainTheme = useMemo(
    () => buildAntdAppTheme({ antdTokens, tokens: t, isDark }, 'main'),
    [antdTokens, isDark, t],
  );

  return (
    <ConfigProvider locale={antdLocale} theme={mainTheme}>
      <AntdApp>
        <LocaleSync />
        <WsContext.Provider value={{ ...ws, mainSessionId: ws.mainSessionId }}>
          <Layout />
          <ImagePreview />
        </WsContext.Provider>
      </AntdApp>
    </ConfigProvider>
  );
}

/** 全屏看图：监听 xy-image-preview；点遮罩关闭。 */
function ImagePreview() {
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.src) setSrc(detail.src);
    };
    window.addEventListener('xy-image-preview', handler);
    return () => window.removeEventListener('xy-image-preview', handler);
  }, []);

  if (!src) return null;
  return (
    <div
      className="xy-image-preview-overlay"
      onClick={() => setSrc(null)}
      role="presentation"
    >
      <img src={src} alt="" onClick={(e) => e.stopPropagation()} />
    </div>
  );
}
