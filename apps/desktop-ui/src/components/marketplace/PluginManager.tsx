/**
 * 插件市场：插件与频道类条目（不含 MCP / Skill）。
 */
import MarketplaceHub from './MarketplaceHub';
import type { MarketplaceCatalogKind } from '../../stores/marketplaceStore';

const PLUGIN_KINDS: MarketplaceCatalogKind[] = ['plugin'];

/** 被谁调用：市场 Hub。做什么：插件 scope 的 MarketplaceHub 包装。 */
export default function PluginManager({ active = true }: { active?: boolean }) {
  return <MarketplaceHub kinds={PLUGIN_KINDS} i18nPrefix="plugins" active={active} />;
}
