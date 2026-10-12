/**
 * 引擎进程状态条：退出/错误时提示并可一键重启。
 */

import React, { useCallback, useEffect, useState } from 'react';
import { Alert, Button, Space } from 'antd';

type EngineStatus = {
  state: 'stopped' | 'starting' | 'ready' | 'error';
  version?: string;
  detail?: string;
};

export default function EngineStatusBanner() {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [restarting, setRestarting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void window.xuanyu?.getEngineStatus?.().then((next) => {
      if (!cancelled) setStatus(next);
    });
    const unsub = window.xuanyu?.onEvent?.('engine.status', (payload) => {
      setStatus(payload as EngineStatus);
    });
    return () => {
      cancelled = true;
      unsub?.();
    };
  }, []);

  const restart = useCallback(async () => {
    if (!window.xuanyu?.restartEngine) return;
    setRestarting(true);
    try {
      const next = await window.xuanyu.restartEngine();
      setStatus(next);
      // useWebSocket 监听 engine.status → ready 会 reconnect；此处再保险触发一次。
      if (next.state === 'ready') {
        window.dispatchEvent(new CustomEvent('xuanyu:engine-reconnected'));
      }
    } catch (err) {
      setStatus({
        state: 'error',
        detail: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setRestarting(false);
    }
  }, []);

  if (!status || status.state === 'ready' || status.state === 'starting') {
    return null;
  }

  const message =
    status.state === 'stopped'
      ? '本地引擎已停止'
      : `本地引擎异常：${status.detail || status.state}`;

  return (
    <Alert
      type={status.state === 'error' ? 'error' : 'warning'}
      showIcon
      banner
      message={
        <Space>
          <span>{message}</span>
          <Button size="small" type="primary" loading={restarting} onClick={() => void restart()}>
            重启引擎
          </Button>
        </Space>
      }
    />
  );
}
