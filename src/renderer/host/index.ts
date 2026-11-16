// 렌더러가 호스트(Tauri)에 접근하는 유일한 통로. app.ts 와 플러그인은 peek URL 규칙이나 Tauri API 를 직접 쓰지 않고
// 이 어댑터만 거친다(docs/tauri-migration.md 3.2, 3.6).

import type { PeekabooApi } from '../../shared/preview-api'
import type { FileOps } from './file-ops/index'
import { createTauriHost } from './tauri'

/**
 * 렌더러에 주는 호스트 API.
 * - PeekabooApi: 호스트와의 IPC(표시 이벤트, 창 맞춤, 네이티브 미리보기, 폴더 목록)와 `fileUrl`(현재 표시 중인 파일의 URL)
 * - FileOps: 파일 해석. 바이트는 `fileUrl` 의 Range 요청으로만 읽는다
 */
export interface Host extends PeekabooApi, FileOps {}

/** 렌더러에 호스트 API 를 준다. 호스트는 Tauri(src-tauri) 하나다. */
export function createHost(): Host {
  return createTauriHost()
}
