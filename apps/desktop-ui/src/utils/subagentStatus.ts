/** 子 Agent / 工具状态文案与 Tag 色（侧栏任务列表已删除，仅作通用状态映射）。 */

export function formatSubAgentStatus(status: string, locale: 'zh-CN' | 'en' = 'zh-CN'): string {
  const labels: Record<string, [string, string]> = {
    running: ['运行中', 'Running'],
    completed: ['已完成', 'Completed'],
    failed: ['失败', 'Failed'],
    blocked: ['已阻塞', 'Blocked'],
    queued: ['排队中', 'Queued'],
    calling: ['调用中', 'Calling'],
    done: ['已完成', 'Done'],
    error: ['错误', 'Error'],
    pending: ['等待中', 'Pending'],
    cancelled: ['已取消', 'Cancelled'],
  };
  const label = labels[status];
  if (label) return label[locale === 'zh-CN' ? 0 : 1];
  return status;
}

/** Ant Design Tag 色：运行 processing、成功 green、失败 red、排队 gold，其余 default。 */
export function subAgentStatusColor(status: string): string {
  if (status === 'running' || status === 'calling') return 'processing';
  if (status === 'completed' || status === 'done') return 'green';
  if (status === 'failed' || status === 'blocked' || status === 'error') return 'red';
  if (status === 'queued') return 'gold';
  if (status === 'cancelled') return 'default';
  return 'default';
}
