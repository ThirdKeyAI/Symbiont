import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [tailwindcss()],
  build: {
    outDir: 'dist',
    target: 'es2022',
    // Don't emit the inline modulePreload-polyfill <script>; the es2022 target
    // already excludes browsers that need it. Keeps the built index.html free
    // of inline scripts so a strict `script-src 'self'` CSP doesn't break it.
    modulePreload: { polyfill: false },
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/ws': {
        target: 'ws://localhost:8080',
        ws: true,
      },
    },
  },
});
