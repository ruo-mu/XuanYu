import path from 'path';
import { fileURLToPath } from 'url';

import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const devHost = process.env.HOST || '127.0.0.1';
const devPort = Number(process.env.PORT || 5173);

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
  base: './',
  plugins: [react(), removeCrossoriginPlugin()],
  resolve: {
    alias: {
      '@xuanyu/contracts': path.resolve(__dirname, 'src/foundation/contracts'),
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    modulePreload: { polyfill: false },
  },
  server: {
    host: devHost,
    port: devPort,
    strictPort: true,
    hmr: {
      host: devHost,
    },
  },
});
