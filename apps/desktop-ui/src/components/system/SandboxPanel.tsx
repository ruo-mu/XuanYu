/**
 * SandboxPanel — 只保留可改行为的沙箱开关（去掉只读说明堆与空进程表）。
 */

import React from 'react';
import { Form, Switch } from 'antd';

import { useLocaleText } from '../../i18n';
import type { AppConfig } from '../../stores/configStore';
import { DataBlock, PageSurface } from '../shell/PanelShell';

/** 被谁调用：系统设置。做什么：沙箱配置读写。 */
export default function SandboxPanel({
  config,
  onChange,
  embedded,
}: {
  config?: AppConfig | null;
  onChange?: (config: AppConfig) => void;
  embedded?: boolean;
}) {
  const lt = useLocaleText();
  if (!config || !onChange) return null;

  const sandbox = config.sandbox || {
    enabled: true,
    cwdAllowlist: [] as string[],
    autoForRiskyCommands: true,
  };

  const patch = (next: AppConfig['sandbox']) => {
    onChange({ ...config, sandbox: next });
  };

  const content = (
    <PageSurface>
      <DataBlock title={lt('执行隔离', 'Execution isolation')}>
        <Form layout="vertical" className="xy-settings-form">
          <Form.Item
            label={lt('危险命令自动 worktree', 'Auto worktree for risky exec')}
            extra={lt(
              '开启后，判定为高风险的 shell 在 git 仓库中会自动创建隔离 worktree；非 git 目录则仍在原工作区执行。',
              'When on, risky shell commands auto-create an isolated git worktree in a repo; non-git dirs stay in the workspace.',
            )}
          >
            <Switch
              checked={sandbox.autoForRiskyCommands !== false}
              onChange={(autoForRiskyCommands) =>
                patch({
                  ...sandbox,
                  autoForRiskyCommands,
                })
              }
            />
          </Form.Item>
          <Form.Item
            label={lt('启用 OS 沙箱', 'Enable OS sandbox')}
            extra={lt(
              '开启后用系统级隔离执行命令；本机后端不可用则拒绝执行（不裸跑）。macOS 需 sandbox-exec，Linux 需 bwrap；Windows 当前无后端，开启即拒绝。默认关闭。',
              'When on, wrap commands in an OS sandbox; if the backend cannot attach, reject (no bare run). Needs sandbox-exec on macOS, bwrap on Linux; Windows has no backend yet. Default off.',
            )}
          >
            <Switch
              checked={sandbox.osSandbox?.enabled === true}
              onChange={(enabled) =>
                patch({
                  ...sandbox,
                  osSandbox: {
                    ...(sandbox.osSandbox || {}),
                    enabled,
                  },
                })
              }
            />
          </Form.Item>
          <Form.Item
            label={lt('沙箱内断网', 'Disable network inside sandbox')}
            extra={lt(
              'OS 沙箱内默认断网，降低工具外联面。仅在 OS 沙箱启用时生效。',
              'Disable network inside the OS sandbox by default. Only applies when OS sandbox is on.',
            )}
          >
            <Switch
              checked={sandbox.osSandbox?.networkOff !== false}
              disabled={sandbox.osSandbox?.enabled !== true}
              onChange={(networkOff) =>
                patch({
                  ...sandbox,
                  osSandbox: {
                    ...(sandbox.osSandbox || {}),
                    enabled: true,
                    networkOff,
                  },
                })
              }
            />
          </Form.Item>
        </Form>
      </DataBlock>
    </PageSurface>
  );

  if (embedded) return content;
  return <div className="xy-panel-page">{content}</div>;
}
