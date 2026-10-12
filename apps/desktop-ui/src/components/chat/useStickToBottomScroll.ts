/**
 * Stick-to-bottom scroll: follow new content only while the user is near the bottom.
 * Scrolling up to read history pauses auto-follow until they return near the end.
 */

import { useCallback, useLayoutEffect, useRef, type DependencyList, type UIEventHandler } from 'react';

const DEFAULT_NEAR_BOTTOM_PX = 120;

/** 被谁调用：粘底滚动。做什么：判断是否接近底部。 */
export function isNearBottom(el: HTMLElement, thresholdPx = DEFAULT_NEAR_BOTTOM_PX): boolean {
  return el.scrollHeight - el.scrollTop - el.clientHeight <= thresholdPx;
}

/** 被谁调用：ChatView。做什么：流式输出时粘底，用户上滚则暂停。 */
export function useStickToBottomScroll(
  deps: DependencyList,
  options?: { thresholdPx?: number },
) {
  const scrollerRef = useRef<HTMLDivElement>(null);
  const stickRef = useRef(true);
  const thresholdPx = options?.thresholdPx ?? DEFAULT_NEAR_BOTTOM_PX;

  const onScroll = useCallback<UIEventHandler<HTMLDivElement>>(() => {
    const el = scrollerRef.current;
    if (!el) return;
    stickRef.current = isNearBottom(el, thresholdPx);
  }, [thresholdPx]);

  /** Force follow (e.g. after the user sends a message). */
  const pinToBottom = useCallback(() => {
    stickRef.current = true;
    const el = scrollerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, []);

  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el || !stickRef.current) return;
    el.scrollTop = el.scrollHeight;
    // Intentionally keyed by caller deps (messages / stream chunks).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return { scrollerRef, onScroll, pinToBottom };
}
