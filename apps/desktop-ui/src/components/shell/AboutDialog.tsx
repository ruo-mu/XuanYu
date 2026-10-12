/**
 * 关于玄语：应用内自定义关于面板（替代系统 alert）。
 */

import React from 'react';
import { Modal } from 'antd';
import { useI18n } from '../../i18n';
import { BrandGlyph } from '../brand/BrandMark';

const APP_VERSION = '0.1.0-preview';

export interface AboutDialogProps {
  open: boolean;
  onClose: () => void;
}

/** 被谁调用：Layout 菜单「关于」。做什么：展示品牌、版本与产品简介。 */
export default function AboutDialog({ open, onClose }: AboutDialogProps) {
  const { t } = useI18n();

  return (
    <Modal
      open={open}
      onCancel={onClose}
      footer={null}
      centered
      width={420}
      destroyOnHidden
      className="xy-about-modal"
      styles={{
        content: {
          padding: 0,
          overflow: 'hidden',
          borderRadius: 18,
          background: 'transparent',
          boxShadow: 'none',
        },
        body: { padding: 0 },
        mask: { backdropFilter: 'blur(8px)' },
      }}
      closable={false}
    >
      <div className="xy-about">
        <div className="xy-about-glow" aria-hidden />
        <button
          type="button"
          className="xy-about-close"
          onClick={onClose}
          aria-label={t('titlebar.help.aboutClose', '关闭')}
        >
          ×
        </button>

        <div className="xy-about-mark" aria-hidden>
          <BrandGlyph size={34} />
        </div>

        <h2 className="xy-about-title">{t('app.title', '玄语')}</h2>
        <p className="xy-about-tagline">
          {t('titlebar.help.aboutTagline', '本地桌面智能体工作台')}
        </p>

        <div className="xy-about-meta">
          <span className="xy-about-chip">v{APP_VERSION}</span>
          <span className="xy-about-chip">
            {t('titlebar.help.aboutEdition', 'Desktop')}
          </span>
        </div>

        <p className="xy-about-desc">
          {t(
            'titlebar.help.aboutDesc',
            '在本地完成对话、项目协作与长期记忆沉淀，让智能体真正落在你的工作区里。',
          )}
        </p>

        <button type="button" className="xy-about-ok" onClick={onClose}>
          {t('titlebar.help.aboutOk', '知道了')}
        </button>
      </div>
    </Modal>
  );
}
