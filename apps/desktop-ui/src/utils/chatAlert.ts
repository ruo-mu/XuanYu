/**
 * 运行时提示：超时 / 报错 / 停止等不进聊天时间线，改用 antd 浮层。
 * Modal.error ≈ Element Plus MessageBox；轻量状态用 message。
 */
import { createElement } from 'react';
import { Modal, message } from 'antd';

/** 读 xy-locale：en 返回英文，否则中文。 */
function localeText(zh: string, en: string): string {
  try {
    return window.localStorage.getItem('xy-locale') === 'en' ? en : zh;
  } catch {
    return zh;
  }
}

/** Modal 内容：保留换行与长词断行。 */
function preWrap(content: string) {
  return createElement('div', { style: { whiteSpace: 'pre-wrap', wordBreak: 'break-word' } }, content);
}

/** 报错 / 超时：MessageBox 式确认框 */
export function notifyChatError(content: string, title?: string) {
  const text = (content || '').trim();
  if (!text) return;
  Modal.error({
    title: title || localeText('出错了', 'Something went wrong'),
    content: preWrap(text),
    okText: localeText('知道了', 'OK'),
  });
}

/** 手动停止等非错误状态：顶部轻提示 */
export function notifyChatInfo(content: string) {
  const text = (content || '').trim();
  if (!text) return;
  message.info(text);
}
