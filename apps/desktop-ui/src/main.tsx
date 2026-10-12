/**
 * main.tsx - Web 前端入口
 *
 * 用途：加载全局样式，并把 React 应用挂载到页面根节点上。
 * 使用 StrictMode 帮助检测潜在问题。
 */

import './styles/styles.css';
import './styles/glass.css';
import './styles/agentTeam.css';
import './styles/electron-chrome.css';

import React from 'react';

import ReactDOM from 'react-dom/client';

import App from './App';
import { bootstrapElectronChrome } from './utils/electronChrome';

bootstrapElectronChrome();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
