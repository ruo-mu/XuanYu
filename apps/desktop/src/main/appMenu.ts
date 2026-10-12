import { BrowserWindow, Menu, app, type MenuItemConstructorOptions } from 'electron';
import { IPC_CHANNEL } from '../shared/protocol';

export type AppMenuId = 'file' | 'edit' | 'view' | 'help';

const menus: Partial<Record<AppMenuId, Menu>> = {};

function sendAction(win: BrowserWindow | null, type: string, payload: Record<string, unknown> = {}) {
  if (!win || win.isDestroyed()) return;
  win.webContents.send(IPC_CHANNEL.menuAction, { type, ...payload });
}

/** 构建应用菜单（快捷键与 macOS 菜单栏；Windows 标题栏用自定义浮层）。 */
export function buildAppMenus(getWindow: () => BrowserWindow | null): void {
  const win = () => getWindow();

  const fileTemplate: MenuItemConstructorOptions[] = [
    {
      label: '新建对话',
      accelerator: 'CmdOrCtrl+N',
      click: () => sendAction(win(), 'new-chat'),
    },
    {
      label: '打开文件夹',
      accelerator: 'CmdOrCtrl+O',
      click: () => sendAction(win(), 'open-folder'),
    },
    { type: 'separator' },
    {
      label: '系统设置',
      click: () => sendAction(win(), 'navigate', { page: 'system' }),
    },
    {
      label: '配置向导',
      click: () => sendAction(win(), 'open-setup'),
    },
    { type: 'separator' },
    {
      label: '关闭',
      accelerator: 'CmdOrCtrl+W',
      click: () => {
        const w = win();
        if (w && !w.isDestroyed()) w.close();
      },
    },
    // Windows/Linux 上 role:quit 无默认快捷键，需显式指定（macOS 默认即 Cmd+Q）
    { role: 'quit', label: '退出玄语', accelerator: 'CmdOrCtrl+Q' },
  ];

  const editTemplate: MenuItemConstructorOptions[] = [
    { role: 'undo', label: '撤销' },
    { role: 'redo', label: '重做' },
    { type: 'separator' },
    { role: 'cut', label: '剪切' },
    { role: 'copy', label: '复制' },
    { role: 'paste', label: '粘贴' },
    { role: 'delete', label: '删除' },
    { type: 'separator' },
    { role: 'selectAll', label: '全选' },
    { type: 'separator' },
    {
      label: '设置',
      accelerator: 'CmdOrCtrl+,',
      click: () => sendAction(win(), 'navigate', { page: 'system' }),
    },
  ];

  const viewTemplate: MenuItemConstructorOptions[] = [
    {
      label: '切换侧边栏',
      accelerator: 'CmdOrCtrl+B',
      click: () => sendAction(win(), 'toggle-sidebar'),
    },
    {
      label: '切换终端',
      accelerator: 'CmdOrCtrl+`',
      click: () => sendAction(win(), 'toggle-terminal'),
    },
    { type: 'separator' },
    {
      label: '主题设置',
      click: () => sendAction(win(), 'open-theme'),
    },
    {
      label: '切换浅色/深色',
      click: () => sendAction(win(), 'toggle-theme-mode'),
    },
    {
      label: '切换语言',
      click: () => sendAction(win(), 'toggle-locale'),
    },
    { type: 'separator' },
    { role: 'zoomIn', label: '放大', accelerator: 'CmdOrCtrl+Shift+=' },
    { role: 'zoomOut', label: '缩小' },
    { role: 'resetZoom', label: '实际大小' },
    { type: 'separator' },
    { role: 'togglefullscreen', label: '切换全屏' },
    { role: 'reload', label: '重新加载' },
    ...(app.isPackaged ? [] : [{ role: 'toggleDevTools' as const, label: '开发者工具' }]),
  ];

  const helpTemplate: MenuItemConstructorOptions[] = [
    {
      label: '关于玄语',
      click: () => sendAction(win(), 'about'),
    },
  ];

  menus.file = Menu.buildFromTemplate(fileTemplate);
  menus.edit = Menu.buildFromTemplate(editTemplate);
  menus.view = Menu.buildFromTemplate(viewTemplate);
  menus.help = Menu.buildFromTemplate(helpTemplate);

  const template: MenuItemConstructorOptions[] =
    process.platform === 'darwin'
      ? [
          {
            label: '玄语',
            submenu: [
              {
                label: '关于玄语',
                click: () => sendAction(win(), 'about'),
              },
              { type: 'separator' },
              { role: 'services' },
              { type: 'separator' },
              { role: 'hide' },
              { role: 'hideOthers' },
              { role: 'unhide' },
              { type: 'separator' },
              { role: 'quit', label: '退出玄语' },
            ],
          },
          { label: '文件', submenu: fileTemplate },
          { label: '编辑', submenu: editTemplate },
          { label: '视图', submenu: viewTemplate },
          { label: '帮助', submenu: helpTemplate },
        ]
      : [
          { label: '文件', submenu: fileTemplate },
          { label: '编辑', submenu: editTemplate },
          { label: '视图', submenu: viewTemplate },
          { label: '帮助', submenu: helpTemplate },
        ];

  // Windows / Linux：菜单栏隐藏，快捷键仍生效；可见菜单走标题栏浮层。
  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

export function popupAppMenu(menuId: AppMenuId, position: { x: number; y: number }): void {
  const menu = menus[menuId];
  if (!menu) return;
  const focused = BrowserWindow.getFocusedWindow();
  menu.popup({
    window: focused ?? undefined,
    x: Math.round(position.x),
    y: Math.round(position.y),
  });
}

export function runWindowCommand(
  getWindow: () => BrowserWindow | null,
  command: string,
): { ok: boolean; maximized?: boolean } {
  const win = getWindow();
  if (!win || win.isDestroyed()) return { ok: false };

  switch (command) {
    case 'quit':
      app.quit();
      return { ok: true };
    case 'close':
      win.close();
      return { ok: true };
    case 'minimize':
      win.minimize();
      return { ok: true, maximized: win.isMaximized() };
    case 'maximize':
      win.maximize();
      return { ok: true, maximized: win.isMaximized() };
    case 'unmaximize':
      win.unmaximize();
      return { ok: true, maximized: win.isMaximized() };
    case 'toggleMaximize':
      if (win.isMaximized()) win.unmaximize();
      else win.maximize();
      return { ok: true, maximized: win.isMaximized() };
    case 'zoomIn':
      win.webContents.setZoomLevel(win.webContents.getZoomLevel() + 0.5);
      return { ok: true };
    case 'zoomOut':
      win.webContents.setZoomLevel(win.webContents.getZoomLevel() - 0.5);
      return { ok: true };
    case 'resetZoom':
      win.webContents.setZoomLevel(0);
      return { ok: true };
    case 'toggleFullscreen':
      win.setFullScreen(!win.isFullScreen());
      return { ok: true };
    case 'reload':
      win.webContents.reload();
      return { ok: true };
    default:
      return { ok: false };
  }
}
