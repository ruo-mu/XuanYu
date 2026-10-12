/**
 * MCP 市场：独立菜单，布局与插件中心一致。
 */
import MarketplaceHub from './MarketplaceHub';
import type { MarketplaceCatalogKind } from '../../stores/marketplaceStore';

const MCP_KINDS: MarketplaceCatalogKind[] = ['mcp'];

/** MCP 市场页：MarketplaceHub 限定 kind=mcp。 */
export default function McpManager({ active = true }: { active?: boolean }) {
  return <MarketplaceHub kinds={MCP_KINDS} i18nPrefix="mcp" active={active} />;
}
