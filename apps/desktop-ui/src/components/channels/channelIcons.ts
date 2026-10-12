/**
 * 频道品牌图标表：按渠道 type 取 Iconify 图标，未知类型回落到 webhook。
 */

import type { IconifyIcon } from '@iconify/types';
import discord from '@iconify-icons/simple-icons/discord';
import qq from '@iconify-icons/simple-icons/tencentqq';
import wechat from '@iconify-icons/simple-icons/wechat';
import telegram from '@iconify-icons/simple-icons/telegram';
import whatsapp from '@iconify-icons/simple-icons/whatsapp';
import line from '@iconify-icons/simple-icons/line';
import dingtalk from '@iconify-icons/mingcute/dingtalk-fill';
import lark from '@iconify-icons/icon-park/lark';
import webhook from '@iconify-icons/mdi/webhook';

/** 按需导入的 Iconify 品牌图标（第三方包，禁止手绘）。 */
export const CHANNEL_BRAND_ICONS: Record<string, IconifyIcon> = {
  qq,
  wechat,
  telegram,
  discord,
  feishu: lark,
  dingtalk,
  whatsapp,
  line,
};

/** 按渠道 type 取品牌图标；未登记时用通用 webhook 图标。 */
export function channelBrandIcon(type: string): IconifyIcon {
  return CHANNEL_BRAND_ICONS[type] || webhook;
}
