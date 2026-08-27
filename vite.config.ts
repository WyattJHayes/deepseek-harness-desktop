import process from 'node:process'
import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

const host = process.env.TAURI_DEV_HOST

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [
    react({
      babel: {
        plugins: [['babel-plugin-react-compiler', { target: '19' }]],
      },
    }),
    tailwindcss(),
  ],

  resolve: {
    alias: {
      '@': '/src',
    },
  },

  build: {
    rollupOptions: {
      output: {
        // 将稳定的第三方运行时拆开，避免入口 chunk 同时承载应用代码和所有依赖。
        manualChunks(id) {
          if (!id.includes('/node_modules/'))
            return undefined
          if (id.includes('/node_modules/react-dom/'))
            return 'vendor-react-dom'
          if (id.includes('/node_modules/react/'))
            return 'vendor-react'
          if (
            id.includes('/node_modules/react-aria/')
            || id.includes('/node_modules/react-aria-components/')
            || id.includes('/node_modules/react-stately/')
          ) {
            return 'vendor-react-aria'
          }
          if (id.includes('/node_modules/@heroui/'))
            return 'vendor-heroui'
          if (id.includes('/node_modules/@gravity-ui/'))
            return 'vendor-icons'
          if (id.includes('/node_modules/@tanstack/'))
            return 'vendor-query'
          if (id.includes('/node_modules/@tauri-apps/'))
            return 'vendor-tauri'
          if (id.includes('/node_modules/@overlastic/'))
            return 'vendor-overlays'
          if (
            id.includes('/node_modules/@hairy/')
            || id.includes('/node_modules/bignumber.js/')
            || id.includes('/node_modules/valtio')
          ) {
            return 'vendor-hairy'
          }
          if (id.includes('/node_modules/i18next/'))
            return 'vendor-i18n'
          if (id.includes('/node_modules/tailwind-variants/'))
            return 'vendor-styling'
          return undefined
        },
      },
    },
  },

  // Vite options tailored for Tauri development.
  // 1. prevent vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell vite to ignore watching `src-tauri`
      ignored: ['**/src-tauri/**'],
    },
  },
}))
