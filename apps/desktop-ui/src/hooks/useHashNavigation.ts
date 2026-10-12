/**
 * Hash 路由前进/后退栈（集成标题栏用）。
 */

import { useCallback, useEffect, useRef, useState } from 'react';

function currentHash(): string {
  return window.location.hash || '#/chat';
}

export function useHashNavigation() {
  const stackRef = useRef<string[]>([currentHash()]);
  const indexRef = useRef(0);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);

  const syncFlags = useCallback(() => {
    setCanGoBack(indexRef.current > 0);
    setCanGoForward(indexRef.current < stackRef.current.length - 1);
  }, []);

  const pushHash = useCallback(
    (hash: string) => {
      const normalized = hash.startsWith('#') ? hash : `#${hash}`;
      if (stackRef.current[indexRef.current] === normalized) return;
      stackRef.current = stackRef.current.slice(0, indexRef.current + 1);
      stackRef.current.push(normalized);
      indexRef.current = stackRef.current.length - 1;
      syncFlags();
    },
    [syncFlags],
  );

  useEffect(() => {
    const onHashChange = () => {
      const hash = currentHash();
      const top = stackRef.current[indexRef.current];
      if (hash === top) return;
      const existing = stackRef.current.lastIndexOf(hash);
      if (existing >= 0) {
        indexRef.current = existing;
      } else {
        pushHash(hash);
        return;
      }
      syncFlags();
    };
    window.addEventListener('hashchange', onHashChange);
    return () => window.removeEventListener('hashchange', onHashChange);
  }, [pushHash, syncFlags]);

  const goBack = useCallback(() => {
    if (indexRef.current <= 0) return;
    indexRef.current -= 1;
    window.location.hash = stackRef.current[indexRef.current].replace(/^#/, '');
    syncFlags();
  }, [syncFlags]);

  const goForward = useCallback(() => {
    if (indexRef.current >= stackRef.current.length - 1) return;
    indexRef.current += 1;
    window.location.hash = stackRef.current[indexRef.current].replace(/^#/, '');
    syncFlags();
  }, [syncFlags]);

  return { canGoBack, canGoForward, goBack, goForward, pushHash };
}
