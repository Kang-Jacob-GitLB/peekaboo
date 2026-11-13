import { resolve } from 'node:path'
import { defineConfig } from 'electron-vite'
import { hostDefine, pdfjsAssets } from './vite.shared'

export default defineConfig({
  main: {
    build: { rollupOptions: { input: { index: resolve(__dirname, 'src/main/index.ts') } } }
  },
  preload: {
    build: { rollupOptions: { input: { index: resolve(__dirname, 'src/preload/index.ts') } } }
  },
  renderer: {
    root: resolve(__dirname, 'src/renderer'),
    plugins: [pdfjsAssets()],
    define: hostDefine('electron'),
    build: {
      rollupOptions: {
        input: { index: resolve(__dirname, 'src/renderer/index.html') },
        // host/tauri.ts 는 VITE_HOST 상수 때문에 번들에서 빠지지만, @tauri-apps/api 모듈의 최상위 문(enum 초기화)은 남는다.
        // Electron 번들에는 Tauri 코드가 하나도 들어가지 않게 부작용 없는 모듈로 취급한다.
        treeshake: { moduleSideEffects: (id: string) => !/[\\/]@tauri-apps[\\/]/.test(id) }
      }
    }
  }
})
