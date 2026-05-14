import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';
import { resolve } from 'node:path';

export default defineConfig({
  plugins: [react()],
  base: '/playground/',
  server: {
    fs: {
      allow: [path.resolve(__dirname, '../..')],
    },
  },
  resolve: {
    dedupe: ['react', 'react-dom'],
  },
  define: {
    __BUILD_TIME__: JSON.stringify(Date.now()),
  },
  build: {
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        legacy: resolve(__dirname, 'src/legacy/legacy.html'),
      },
    },
  },
});
