import type { PeekabooApi } from '../shared/preview-api'

declare global {
  interface Window {
    peekaboo: PeekabooApi
  }

  // 빌드 대상. vite 설정(vite.shared.ts 의 hostDefine)이 컴파일 타임 상수로 박는다.
  interface ImportMetaEnv {
    readonly VITE_HOST: 'electron' | 'tauri'
  }
  interface ImportMeta {
    readonly env: ImportMetaEnv
  }
}

export {}
