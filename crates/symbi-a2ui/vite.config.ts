import { defineConfig } from 'vite';
import tailwindcss from '@tailwindcss/vite';

const runtimeUrl = process.env.SYMBI_RUNTIME_URL ?? 'http://localhost:8080';

export default defineConfig({
  plugins: [tailwindcss()],
  // Dependencies and the production bundle support the same browser baseline.
  optimizeDeps: { esbuildOptions: { target: 'es2022' } },
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
        target: runtimeUrl,
        changeOrigin: true,
      },
      '/ws': {
        target: runtimeUrl,
        ws: true,
      },
    },
  },
});
