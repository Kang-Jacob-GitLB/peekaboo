// Electron 호스트: preload 가 contextBridge 로 노출한 window.peekaboo 를 감싼다.
// 파일 해석은 main 에 맡기지 않고 peek:// Range 요청 위에서 렌더러가 한다.

import type { PeekabooApi } from '../../shared/preview-api'
import { createFileOps } from './file-ops/index'
import type { Host } from './index'

export function createElectronHost(api: PeekabooApi = window.peekaboo): Host {
  return { ...api, ...createFileOps((path) => api.fileUrl(path)) }
}
