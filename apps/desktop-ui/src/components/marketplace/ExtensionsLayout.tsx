/**
 * Extensions hub: plugins / MCP / skills panels (section nav lives in AppSidebar).
 */

import React, { useEffect, useState } from 'react';
import {
  extensionsTabFromLocation,
  type ExtensionsTab,
} from '../../navigation/pageRegistry';
import McpManager from './McpManager';
import PluginManager from './PluginManager';
import SkillsManager from './SkillsManager';

/** 被谁调用：Layout。做什么：扩展页内容壳 + keep-alive 市场面板。 */
export default function ExtensionsLayout() {
  const [tab, setTab] = useState<ExtensionsTab>(extensionsTabFromLocation);
  const [visited, setVisited] = useState<Partial<Record<ExtensionsTab, true>>>(() => {
    const initial = extensionsTabFromLocation();
    return { [initial]: true };
  });

  useEffect(() => {
    const sync = () => setTab(extensionsTabFromLocation());
    window.addEventListener('hashchange', sync);
    return () => window.removeEventListener('hashchange', sync);
  }, []);

  useEffect(() => {
    setVisited((prev) => (prev[tab] ? prev : { ...prev, [tab]: true }));
  }, [tab]);

  return (
    <div className="xy-workspace-frame xy-workspace-content">
      {(visited.plugins || tab === 'plugins') && (
        <div
          className="xy-workspace-scroll is-flush"
          style={{ display: tab === 'plugins' ? undefined : 'none' }}
          aria-hidden={tab !== 'plugins'}
        >
          <PluginManager active={tab === 'plugins'} />
        </div>
      )}
      {(visited.mcp || tab === 'mcp') && (
        <div
          className="xy-workspace-scroll is-flush"
          style={{ display: tab === 'mcp' ? undefined : 'none' }}
          aria-hidden={tab !== 'mcp'}
        >
          <McpManager active={tab === 'mcp'} />
        </div>
      )}
      {(visited.skills || tab === 'skills') && (
        <div
          className="xy-workspace-scroll is-flush"
          style={{ display: tab === 'skills' ? undefined : 'none' }}
          aria-hidden={tab !== 'skills'}
        >
          <SkillsManager active={tab === 'skills'} />
        </div>
      )}
    </div>
  );
}
