import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';

export default defineConfig({
  plugins: [react()],
  // Default `/` matches the Cloudflare Pages deploy (root). For the legacy
  // Hetzner Caddy mount we used `/playground/` — pass `--base=/playground/`
  // or set `BASE_URL=/playground/` if you ever rebuild that target.
  base: process.env.BASE_URL ?? '/',
  server: {
    fs: {
      allow: [path.resolve(__dirname, '../..')],
    },
    // Accept any Host header so a phone can reach the dev server
    // via `vite dev --host` LAN IP or a cloudflared/ngrok tunnel.
    // Dev only — production build doesn't use the dev server.
    allowedHosts: true,
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
        library: path.resolve(__dirname, 'library.html'),
        validation: path.resolve(__dirname, 'validation.html'),
      },
    },
  },
});
