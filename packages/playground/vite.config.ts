import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';

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
        main: path.resolve(__dirname, 'index.html'),
        ci: path.resolve(__dirname, 'ci.html'),
      },
    },
  },
});
