/**
 * 初始化完成页：展示配置路径、网关地址与 Token，可复制打开。
 */

import React, { useMemo } from 'react';
import {
  CheckCircleFilled,
  CopyOutlined,
  ExportOutlined,
  FileTextOutlined,
  GlobalOutlined,
  KeyOutlined,
  WarningFilled,
} from '@ant-design/icons';
import { Button, message } from 'antd';

import { useT } from '../i18n';

interface InitDonePanelProps {
  path: string;
  url?: string;
  token?: string;
  gatewayStarted?: boolean;
  onDone: () => void;
}

/** 被谁调用：InitDonePanel。做什么：抽出 origin。 */
function parseGatewayUrl(url: string): { origin: string } {
  try {
    const parsed = new URL(url);
    return { origin: parsed.origin };
  } catch {
    return { origin: url };
  }
}

/** 被谁调用：InitDonePanel。做什么：剪贴板复制并 toast。 */
async function copyText(text: string, okMessage: string) {
  try {
    await navigator.clipboard.writeText(text);
    message.success(okMessage);
  } catch {
    message.error('Copy failed');
  }
}

/** 向导保存成功后的结果摘要。 */
export default function InitDonePanel({
  path,
  url,
  token,
  gatewayStarted,
  onDone,
}: InitDonePanelProps) {
  const tr = useT();
  const gateway = useMemo(() => (url ? parseGatewayUrl(url) : null), [url]);
  const accessToken = useMemo(() => {
    if (token) return token;
    return undefined;
  }, [token, url]);
  const webHref = url || (gateway ? `${gateway.origin}/` : undefined);

  const openConsole = () => {
    if (!webHref) return;
    window.location.assign(webHref);
  };

  return (
    <div className="xy-init-done-panel">
      <div className="xy-init-done-panel__glow" aria-hidden />

      <header className="xy-init-done-panel__hero">
        <div
          className={`xy-init-done-panel__icon${gatewayStarted ? ' is-success' : ' is-warning'}`}
          aria-hidden
        >
          {gatewayStarted ? <CheckCircleFilled /> : <WarningFilled />}
        </div>
        <div className="xy-init-done-panel__hero-copy">
          <h1>{tr('init.doneTitle')}</h1>
          <p>
            {gatewayStarted ? tr('init.doneGatewayReady') : tr('init.gatewaySaved')}
          </p>
        </div>
      </header>

      <div className="xy-init-done-panel__grid">
        <article className="xy-init-done-card">
          <div className="xy-init-done-card__head">
            <FileTextOutlined />
            <span>{tr('init.doneConfigLabel')}</span>
          </div>
          <code className="xy-init-done-card__mono">{path}</code>
          <Button
            type="text"
            size="small"
            icon={<CopyOutlined />}
            className="xy-init-done-card__action"
            onClick={() => void copyText(path, tr('init.doneCopied'))}
          >
            {tr('init.doneCopyPath')}
          </Button>
        </article>

        {webHref ? (
          <article className="xy-init-done-card xy-init-done-card--access">
            <div className="xy-init-done-card__head">
              <GlobalOutlined />
              <span>{tr('init.doneWebLabel')}</span>
              <span className={`xy-init-done-badge${gatewayStarted ? ' is-online' : ' is-pending'}`}>
                {gatewayStarted ? tr('init.doneStatusOnline') : tr('init.doneStatusPending')}
              </span>
            </div>
            <div className="xy-init-done-card__url">
              <strong>{webHref}</strong>
            </div>
            {accessToken ? (
              <div className="xy-init-done-token-row">
                <KeyOutlined />
                <code title={accessToken}>{accessToken}</code>
                <Button
                  type="text"
                  size="small"
                  icon={<CopyOutlined />}
                  aria-label={tr('init.doneCopyToken')}
                  onClick={() => void copyText(accessToken, tr('init.doneCopied'))}
                />
              </div>
            ) : null}
            <ul className="xy-init-done-checklist">
              <li className="is-done">{tr('init.doneCheckConfig')}</li>
              <li className={gatewayStarted ? 'is-done' : 'is-pending'}>
                {gatewayStarted ? tr('init.doneCheckGateway') : tr('init.doneCheckGatewayPending')}
              </li>
              <li>{tr('init.doneCheckEnter')}</li>
            </ul>
            <div className="xy-init-done-card__actions">
              <Button type="primary" block icon={<ExportOutlined />} onClick={openConsole}>
                {tr('init.doneOpenWeb')}
              </Button>
              <Button
                block
                icon={<CopyOutlined />}
                onClick={() => void copyText(webHref, tr('init.doneCopied'))}
              >
                {tr('init.doneCopyLink')}
              </Button>
            </div>
          </article>
        ) : (
          <article className="xy-init-done-card xy-init-done-card--placeholder" aria-hidden />
        )}
      </div>

      {/* 向导是主程序里的一个页面，配置完成后回到工作台。 */}
      <div className="xy-init-done-panel__footer">
        <Button block onClick={onDone}>
          {tr('init.doneBackToApp', '返回主界面')}
        </Button>
      </div>
    </div>
  );
}
