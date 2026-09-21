import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'node:path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': path.resolve(__dirname, './src') },
  },
  server: {
    // 5174：避开 csw-task-web 的 5173，两边可以同时开着比对
    port: 5174,
    strictPort: true,
    proxy: {
      // 开发时直连本地跑的采集服务。会话是 HttpOnly cookie，
      // 必须同源才带得上——所以走代理而不是填绝对地址。
      '/api': { target: 'http://127.0.0.1:8090', changeOrigin: false },
    },
  },
})
