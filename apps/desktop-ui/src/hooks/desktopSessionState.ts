/** Shared main-session id for desktop WS/IPC bridge. */
let mainSessionId = '';

export function getMainSessionId(): string {
  return mainSessionId;
}

export function setMainSessionIdValue(sessionId: string): void {
  mainSessionId = sessionId || '';
}
