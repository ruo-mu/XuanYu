/**
 * 主题设置面板：切换明暗、选预设、微调品牌色并预览。
 * 作为标题栏调色盘旁的气泡内容使用。
 */

import React, { useEffect, useMemo, useState } from 'react';
import {
  Button,
  Collapse,
  ColorPicker,
  Segmented,
  Space,
  Typography,
} from 'antd';
import {
  ImportOutlined,
  MoonOutlined,
  ReloadOutlined,
  SunOutlined,
} from '@ant-design/icons';
import type { Color } from 'antd/es/color-picker';

import { useI18n } from '../../i18n';
import {
  DEFAULT_PRESET_ID,
  THEME_PRESETS,
  getPresetById,
  getPresetDescription,
  getPresetLabel,
  useTheme,
} from '../../theme/ThemeContext';
import { getDefaultCustomTheme } from '../../theme/themeDefaults';
import type { CustomThemeStore, ThemeCategoryInput, ThemeMode } from '../../theme/types';
import {
  buildPaletteFromCategories,
  categoriesFromPresetTokens,
  paletteToCssVars,
} from '../../theme/utils';

const { Text, Paragraph } = Typography;

type CategoryKey = keyof ThemeCategoryInput;

const CATEGORY_KEYS: CategoryKey[] = [
  'brand',
  'accent',
  'semantic',
  'text',
  'background',
  'surface',
  'border',
  'ambient',
];

const FIELD_KEYS: Record<CategoryKey, string[]> = {
  brand: ['primary'],
  accent: ['secondary', 'tertiary'],
  semantic: ['success', 'warning', 'danger'],
  text: ['primary', 'secondary', 'disabled'],
  background: ['base', 'elevated', 'muted'],
  surface: ['glass', 'glassStrong'],
  border: ['default', 'strong'],
  ambient: ['orb1', 'orb2', 'orb3'],
};

function ColorField({
  label,
  value,
  onChange,
  allowAlpha = false,
}: {
  label: string;
  value: string;
  onChange: (color: string) => void;
  allowAlpha?: boolean;
}) {
  const handleChange = (color: Color) => {
    onChange(allowAlpha ? color.toRgbString() : color.toHexString());
  };

  return (
    <div className="xy-theme-color-row">
      <Text className="xy-theme-color-label">{label}</Text>
      <ColorPicker
        value={value}
        onChange={handleChange}
        showText
        format={allowAlpha ? 'rgb' : 'hex'}
      />
    </div>
  );
}

function ThemePreview({ categories, mode }: { categories: ThemeCategoryInput; mode: ThemeMode }) {
  const vars = useMemo(
    () => paletteToCssVars(buildPaletteFromCategories(categories, mode), mode),
    [categories, mode],
  );

  const style = vars as React.CSSProperties;

  return (
    <div className="xy-theme-live-preview" style={style}>
      <div className="xy-theme-live-preview-bg" />
      <div className="xy-theme-live-preview-card">
        <div className="xy-theme-live-preview-title">Preview</div>
        <div className="xy-theme-live-preview-text">Secondary text layer</div>
        <div className="xy-theme-live-preview-actions">
          <span className="xy-theme-live-preview-btn">Primary</span>
          <span className="xy-theme-live-preview-tag">Accent</span>
        </div>
        <div className="xy-theme-live-preview-semantic">
          <span className="is-success">Success</span>
          <span className="is-warning">Warning</span>
          <span className="is-danger">Error</span>
        </div>
      </div>
    </div>
  );
}

export interface ThemeSettingsPanelProps {
  open?: boolean;
}

/** 主题气泡面板内容（不含弹出层容器）。 */
export default function ThemeSettingsPanel({ open = true }: ThemeSettingsPanelProps) {
  const { locale, t } = useI18n();
  const { mode, presetId, custom, setPreset, setCustomTheme, resetCustom } = useTheme();
  const [tab, setTab] = useState<'presets' | 'custom'>(presetId === 'custom' ? 'custom' : 'presets');
  const [editMode, setEditMode] = useState<ThemeMode>(mode);
  const [draft, setDraft] = useState<CustomThemeStore>(custom || getDefaultCustomTheme());

  useEffect(() => {
    if (open) {
      setEditMode(mode);
      if (custom) setDraft(custom);
      setTab(presetId === 'custom' ? 'custom' : 'presets');
    }
  }, [open, custom, mode, presetId]);

  const currentCategories = draft[editMode];

  const setField = (category: CategoryKey, field: string, value: string) => {
    setDraft((prev) => ({
      ...prev,
      [editMode]: {
        ...prev[editMode],
        [category]: {
          ...prev[editMode][category],
          [field]: value,
        },
      },
    }));
  };

  const applyCustom = () => setCustomTheme(draft);

  const importFromPreset = () => {
    const preset = getPresetById(presetId === 'custom' ? DEFAULT_PRESET_ID : presetId)
      || getPresetById(DEFAULT_PRESET_ID)!;
    setDraft(categoriesFromPresetTokens(preset.light, preset.dark));
  };

  const resetDraftToDefault = () => setDraft(getDefaultCustomTheme());

  const collapseItems = CATEGORY_KEYS.map((cat) => ({
    key: cat,
    label: (
      <div className="xy-theme-collapse-label">
        <strong>{t(`theme.cat.${cat}`)}</strong>
        <span>{t(`theme.cat.${cat}Desc`)}</span>
      </div>
    ),
    children: (
      <div className="xy-theme-collapse-body">
        {FIELD_KEYS[cat].map((field) => (
          <ColorField
            key={field}
            label={t(`theme.field.${cat}.${field}`)}
            value={(currentCategories[cat] as Record<string, string>)[field]}
            onChange={(v) => setField(cat, field, v)}
            allowAlpha={cat === 'surface' || cat === 'border' || cat === 'ambient'}
          />
        ))}
      </div>
    ),
  }));

  return (
    <div className="xy-theme-popover-panel">
      <div className="xy-theme-popover-head">
        <strong>{t('theme.title')}</strong>
        <Button size="small" type="text" icon={<ReloadOutlined />} onClick={resetCustom}>
          {t('theme.reset')}
        </Button>
      </div>

      <div className="xy-theme-section">
        <Segmented
          block
          value={tab}
          onChange={(v) => setTab(v as 'presets' | 'custom')}
          options={[
            { value: 'presets', label: t('theme.presets') },
            { value: 'custom', label: t('theme.custom') },
          ]}
        />
      </div>

      {tab === 'presets' ? (
        <div className="xy-theme-preset-grid">
          {THEME_PRESETS.map((preset) => {
            const active = presetId === preset.id;
            return (
              <button
                key={preset.id}
                type="button"
                className={`xy-theme-preset-card${active ? ' is-active' : ''}`}
                onClick={() => setPreset(preset.id)}
              >
                <div
                  className="xy-theme-preset-preview"
                  style={{
                    background: `linear-gradient(135deg, ${preset.preview[0]}, ${preset.preview[1]}, ${preset.preview[2]})`,
                  }}
                />
                <div className="xy-theme-preset-info">
                  <strong>{getPresetLabel(preset, locale)}</strong>
                  <span>{getPresetDescription(preset, locale)}</span>
                </div>
                {active ? <span className="xy-theme-preset-badge">{t('theme.active')}</span> : null}
              </button>
            );
          })}
        </div>
      ) : (
        <div className="xy-theme-custom">
          <Paragraph type="secondary" className="xy-theme-custom-hint">
            {t('theme.customHint')}
          </Paragraph>

          <div className="xy-theme-section">
            <Text className="xy-theme-section-label">{t('theme.editMode')}</Text>
            <Segmented
              block
              value={editMode}
              onChange={(v) => setEditMode(v as ThemeMode)}
              options={[
                { value: 'light', label: t('header.theme.light'), icon: <SunOutlined /> },
                { value: 'dark', label: t('header.theme.dark'), icon: <MoonOutlined /> },
              ]}
            />
          </div>

          <ThemePreview categories={currentCategories} mode={editMode} />

          <Collapse
            bordered={false}
            className="xy-theme-token-collapse"
            defaultActiveKey={['brand', 'text', 'background', 'surface']}
            items={collapseItems}
          />

          <Space direction="vertical" style={{ width: '100%', marginTop: 12 }} size={8}>
            <Button type="primary" block onClick={applyCustom}>
              {t('theme.applyCustom')}
            </Button>
            <Button block icon={<ImportOutlined />} onClick={importFromPreset}>
              {t('theme.importPreset')}
            </Button>
            <Button block onClick={resetDraftToDefault}>
              {t('theme.resetDraft')}
            </Button>
            <Button block onClick={() => setPreset(DEFAULT_PRESET_ID)}>
              {t('theme.backToPresets')}
            </Button>
          </Space>
        </div>
      )}
    </div>
  );
}
