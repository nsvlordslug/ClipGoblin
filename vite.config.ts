import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    // Dev-only: prevent WebView2/Chromium from caching compiled modules across
    // app/Vite restarts (CLAUDE.md gotcha #14). Without this the webview can
    // serve stale JS, so frontend edits silently never reach the running app.
    {
      name: 'dev-no-store',
      apply: 'serve',
      configureServer(server) {
        server.middlewares.use((_req, res, next) => {
          res.setHeader('Cache-Control', 'no-store')
          next()
        })
      },
    },
  ],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
})
