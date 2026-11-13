// Tauri 용 렌더러 빌드: src/renderer → out/renderer-tauri (src-tauri/tauri.conf.json 의 frontendDist).
// Electron 쪽은 electron.vite.config.ts. 공용 부분은 vite.shared.ts.

import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import { hostDefine, pdfjsAssets, stripCspMeta } from './vite.shared'

export default defineConfig({
  root: resolve(__dirname, 'src/renderer'),
  // 상대 경로: Electron 빌드(file://)와 같은 산출물 모양을 유지한다
  base: './',
  plugins: [pdfjsAssets(), stripCspMeta()],
  define: hostDefine('tauri'),
  build: {
    outDir: resolve(__dirname, 'out/renderer-tauri'),
    emptyOutDir: true,
    rollupOptions: { input: { index: resolve(__dirname, 'src/renderer/index.html') } }
  }
})
