/** 页面是否可见；隐藏标签页时应停掉 rAF / WebGL 循环。 */
export function isPageVisible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden';
}

/** 订阅 visibilitychange；返回取消订阅函数。 */
export function onPageVisibilityChange(listener: () => void): () => void {
  if (typeof document === 'undefined') return () => {};
  const handler = () => listener();
  document.addEventListener('visibilitychange', handler);
  return () => document.removeEventListener('visibilitychange', handler);
}
