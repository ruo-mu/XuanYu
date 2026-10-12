import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { defineConfig, externalizeDepsPlugin } from 'electron-vite';
import react from '@vitejs/plugin-react';
import type { Plugin } from 'vite';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const desktopUiRoot = resolve(__dirname, '../desktop-ui');

function removeCrossoriginPlugin(): Plugin {
  return {
    name: 'remove-crossorigin',
    transformIndexHtml(html: string) {
      return html
        .replace(/<script([^>]*?) crossorigin(?:="[^"]*")?([^>]*)>/gi, '<script$1$2>')
        .replace(/<link([^>]*?) crossorigin(?:="[^"]*")?([^>]*)>/gi, '<link$1$2>');
    },
  };
}

export default defineConfig({
  main: {
    plugins: [externalizeDepsPlugin()],
    resolve: {
      alias: {
        '@shared': resolve('src/shared'),
      },
    },
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
    resolve: {
      alias: {
        '@shared': resolve('src/shared'),
      },
    },
  },
  renderer: {
    root: desktopUiRoot,
    resolve: {
      alias: {
        '@xuanyu/contracts': resolve(desktopUiRoot, 'src/foundation/contracts'),
        '@shared': resolve('src/shared'),
      },
    },
    plugins: [react(), removeCrossoriginPlugin()],
    build: {
      rollupOptions: {
        input: resolve(desktopUiRoot, 'index.html'),
      },
    },
  },
});
