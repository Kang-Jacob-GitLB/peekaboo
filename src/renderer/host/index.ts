// 렌더러가 런타임(Electron 등)에 접근하는 유일한 통로. app.ts 와 플러그인은 window.peekaboo 나 peek URL 규칙을 직접 쓰지 않고
// 이 어댑터만 거친다. 런타임이 바뀌어도 어댑터 구현만 바꾸면 되도록 하기 위함이다(docs/tauri-migration.md 3.2, 3.6).

import type { PeekabooApi } from '../../shared/preview-api'
import type { FileOps } from './file-ops/index'
import { createElectronHost } from './electron'

/**
 * 렌더러에 주는 호스트 API.
 * - PeekabooApi: main 과의 IPC(표시 이벤트, 창 맞춤, 네이티브 미리보기, 폴더 목록)와 `fileUrl`(현재 표시 중인 파일의 URL)
 * - FileOps: 파일 해석. 바이트는 `fileUrl` 의 Range 요청으로만 읽는다
 */
export interface Host extends PeekabooApi, FileOps {}

export function createHost(): Host {
  return createElectronHost()
}
