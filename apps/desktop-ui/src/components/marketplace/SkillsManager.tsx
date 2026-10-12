/**
 * 技能市场：布局与插件中心一致；已安装 Tab 含运行时已加载 skills。
 */
import MarketplaceHub from './MarketplaceHub';
import type { MarketplaceCatalogKind } from '../../stores/marketplaceStore';

const SKILL_KINDS: MarketplaceCatalogKind[] = ['skill'];

/** 技能市场页：MarketplaceHub 限定 kind=skill，含运行时已加载项。 */
export default function SkillsManager({ active = true }: { active?: boolean }) {
  return (
    <MarketplaceHub
      kinds={SKILL_KINDS}
      i18nPrefix="skills"
      includeRuntimeSkills
      active={active}
    />
  );
}
