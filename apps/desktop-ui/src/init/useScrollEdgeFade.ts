/**
 * 滚动容器边缘淡出：根据是否贴顶/贴底给 class，步骤切换时滚回顶部。
 */

import { useEffect, useRef, useState } from 'react';

/** 返回 scrollRef 与 edgeClass（is-scrollable / is-at-top / is-at-bottom）。 */
export function useScrollEdgeFade(activeKey: unknown) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [edgeClass, setEdgeClass] = useState('is-at-top is-at-bottom');
  const prevKeyRef = useRef(activeKey);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;

    const shouldResetScroll = prevKeyRef.current !== activeKey;
    prevKeyRef.current = activeKey;

    const update = () => {
      const { scrollTop, scrollHeight, clientHeight } = el;
      const scrollable = scrollHeight > clientHeight + 2;
      const atTop = scrollTop <= 1;
      const atBottom = scrollTop + clientHeight >= scrollHeight - 2;
      const classes = [
        scrollable ? 'is-scrollable' : '',
        atTop ? 'is-at-top' : '',
        atBottom ? 'is-at-bottom' : '',
      ]
        .filter(Boolean)
        .join(' ');

      setEdgeClass((prev) => (prev === classes ? prev : classes));
    };

    if (shouldResetScroll) {
      el.scrollTop = 0;
    }
    update();

    el.addEventListener('scroll', update, { passive: true });
    const ro = new ResizeObserver(update);
    ro.observe(el);
    for (const child of el.children) ro.observe(child);

    return () => {
      el.removeEventListener('scroll', update);
      ro.disconnect();
    };
  }, [activeKey]);

  return { scrollRef, edgeClass };
}
