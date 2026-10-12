/**
 * Product channel catalog shared by AppSidebar rail and ChannelPanel stage.
 */

export type ChannelTone = 'cyan' | 'indigo' | 'emerald' | 'amber' | 'rose' | 'slate';

export interface ChannelCatalogItem {
  type: string;
  label: string;
  labelEn: string;
  summary: string;
  summaryEn: string;
  tone: ChannelTone;
  qrSupported?: boolean;
  bindMode: 'qr' | 'token' | 'webhook';
}

/**
 * Product catalog. QR bind is currently WeChat iLink only;
 * other platforms use official token / app credentials.
 */
export const CHANNEL_CATALOG: ChannelCatalogItem[] = [
  {
    type: 'wechat',
    label: '微信',
    labelEn: 'WeChat',
    summary: 'iLink 扫码绑定个人号',
    summaryEn: 'iLink QR bind',
    tone: 'emerald',
    qrSupported: true,
    bindMode: 'qr',
  },
  {
    type: 'qq',
    label: 'QQ',
    labelEn: 'QQ',
    summary: '官方 Bot API v2',
    summaryEn: 'Official Bot API v2',
    tone: 'cyan',
    bindMode: 'token',
  },
  {
    type: 'telegram',
    label: 'Telegram',
    labelEn: 'Telegram',
    summary: 'BotFather Token',
    summaryEn: 'BotFather Token',
    tone: 'indigo',
    bindMode: 'token',
  },
  {
    type: 'discord',
    label: 'Discord',
    labelEn: 'Discord',
    summary: 'Bot Gateway',
    summaryEn: 'Bot Gateway',
    tone: 'indigo',
    bindMode: 'token',
  },
  {
    type: 'feishu',
    label: '飞书',
    labelEn: 'Feishu',
    summary: '企业自建应用',
    summaryEn: 'Enterprise app',
    tone: 'cyan',
    bindMode: 'token',
  },
  {
    type: 'dingtalk',
    label: '钉钉',
    labelEn: 'DingTalk',
    summary: '机器人回调',
    summaryEn: 'Bot webhook',
    tone: 'amber',
    bindMode: 'webhook',
  },
  {
    type: 'whatsapp',
    label: 'WhatsApp',
    labelEn: 'WhatsApp',
    summary: 'Cloud API',
    summaryEn: 'Cloud API',
    tone: 'emerald',
    bindMode: 'webhook',
  },
  {
    type: 'line',
    label: 'LINE',
    labelEn: 'LINE',
    summary: 'Messaging API',
    summaryEn: 'Messaging API',
    tone: 'emerald',
    bindMode: 'webhook',
  },
];
