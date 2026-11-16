// 렌더러 빌드: src/renderer → out/renderer (src-tauri/tauri.conf.json 의 frontendDist). 공용 부분은 vite.shared.ts.

import { resolve } from 'node:path'
import { defineConfig } from 'vite'
import { pdfjsAssets } from './vite.shared'

export default defineConfig({
  root: resolve(__dirname, 'src/renderer'),
  // 상대 경로: 산출물을 어느 출처에서 서빙해도 자산을 찾게 한다
  base: './',
  plugins: [pdfjsAssets()],
  build: {
    outDir: resolve(__dirname, 'out/renderer'),
    emptyOutDir: true,
    rollupOptions: { input: { index: resolve(__dirname, 'src/renderer/index.html') } }
  }
})
