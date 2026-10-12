/**
 * 网关 WebSocket 地址（兼容层）：桌面版日常对话走引擎 IPC，
 * 本模块仍被 Auth bootstrap / 部分设置页用于可选远程接入地址解析。
 */

const DEFAULT_WS = 'ws://localhost:18900/ws';

/** localStorage 里网关 URL / Token / 启动时间的键。 */
export const AUTH_STORAGE_KEYS = {
  gatewayUrl: 'xy-gateway-url',
  gatewayToken: 'xy-gateway-token',
  gatewayStartedAt: 'xy-auth-gateway-started-at',
} as const;

/** resolveAuthBootstrap 返回值：当前网关 WS URL。 */
export interface AuthBootstrap {
  gatewayUrl: string;
}

/** 从当前页面 origin 拼 ws(s)://host/ws。 */
export function buildWsUrlFromPage(): string {
  if (typeof window === 'undefined') return DEFAULT_WS;
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/ws`;
}

/** WebSocket URL 转 HTTP origin，供 REST 调用。 */
export function wsUrlToHttpOrigin(wsUrl: string): string {
  try {
    const url = new URL(wsUrl);
    url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
    url.pathname = '/';
    url.search = '';
    url.hash = '';
    return url.origin;
  } catch {
    return '';
  }
}

/** 把 http(s) 换成 ws(s)，缺 path 补 /ws，去掉 query 里的 token。 */
export function normalizeGatewayWsUrl(raw: string): string {
  const value = (raw || '').trim();
  if (!value) return '';
  try {
    const url = new URL(value);
    if (url.protocol === 'http:') url.protocol = 'ws:';
    if (url.protocol === 'https:') url.protocol = 'wss:';
    if (!url.pathname || url.pathname === '/') url.pathname = '/ws';
    url.searchParams.delete('token');
    return url.toString();
  } catch {
    return '';
  }
}

/** 直接进入应用：解析网关地址，无登录/Token 入口页。 */
export function resolveAuthBootstrap(): AuthBootstrap {
  if (typeof window === 'undefined') {
    return { gatewayUrl: DEFAULT_WS };
  }

  window.localStorage.removeItem(AUTH_STORAGE_KEYS.gatewayToken);
  window.sessionStorage.removeItem(AUTH_STORAGE_KEYS.gatewayToken);

  const search = new URLSearchParams(window.location.search);
  const gatewayParam = (search.get('gateway') || '').trim();
  const storedUrl = (window.localStorage.getItem(AUTH_STORAGE_KEYS.gatewayUrl) || '').trim();

  let gatewayUrl: string;
  if (gatewayParam) {
    gatewayUrl = normalizeGatewayWsUrl(gatewayParam) || DEFAULT_WS;
  } else if (import.meta.env.DEV) {
    gatewayUrl = normalizeGatewayWsUrl(storedUrl) || buildWsUrlFromPage();
  } else if (storedUrl) {
    gatewayUrl = normalizeGatewayWsUrl(storedUrl) || DEFAULT_WS;
  } else {
    gatewayUrl = normalizeGatewayWsUrl(buildWsUrlFromPage()) || DEFAULT_WS;
  }

  return { gatewayUrl };
}

/** 把当前网关 URL 写入 localStorage。 */
export function persistAuthSession(payload: {
  gatewayUrl: string;
  token?: string;
  gatewayStartedAt?: number;
}): void {
  window.localStorage.setItem(AUTH_STORAGE_KEYS.gatewayUrl, payload.gatewayUrl);
  window.sessionStorage.removeItem(AUTH_STORAGE_KEYS.gatewayToken);
  if (payload.gatewayStartedAt) {
    window.localStorage.setItem(AUTH_STORAGE_KEYS.gatewayStartedAt, String(payload.gatewayStartedAt));
  }
}

/** 清掉启动时间与 session Token。 */
export function clearAuthSession(): void {
  window.localStorage.removeItem(AUTH_STORAGE_KEYS.gatewayStartedAt);
  window.sessionStorage.removeItem(AUTH_STORAGE_KEYS.gatewayToken);
}

/** 从地址栏去掉 token 查询参数，避免刷新时泄露。 */
export function stripTokenFromLocation(): void {
  const url = new URL(window.location.href);
  if (!url.searchParams.has('token')) return;
  url.searchParams.delete('token');
  window.history.replaceState({}, '', `${url.pathname}${url.search}${url.hash}`);
}

/** 用 /healthz 的 uptime 推算网关进程启动时间。 */
export async function fetchGatewayStartedAt(gatewayWsUrl: string): Promise<number | undefined> {
  const httpOrigin = wsUrlToHttpOrigin(gatewayWsUrl);
  if (!httpOrigin) return undefined;
  try {
    const res = await fetch(`${httpOrigin}/healthz`, { cache: 'no-store' });
    if (!res.ok) return undefined;
    const data = (await res.json()) as { uptime?: number };
    if (typeof data.uptime !== 'number') return undefined;
    return Date.now() - data.uptime * 1000;
  } catch {
    return undefined;
  }
}
