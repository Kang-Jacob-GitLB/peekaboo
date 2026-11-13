// Tauri 호스트: @tauri-apps/api 의 invoke/listen 으로 PeekabooApi 를 구현한다.
// 대응하는 Rust 는 src-tauri/src/{commands,preview}.rs. 허용되는 커맨드·이벤트는 src-tauri/capabilities/preview.json 이 정한다.
// 파일 해석은 Electron 과 같이 peek Range 요청 위에서 렌더러가 한다.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { ContentSize, NativePreviewResult, PeekabooApi } from '../../shared/preview-api'
import { createFileOps } from './file-ops/index'
import type { Host } from './index'

/** Windows(WebView2)에서 peek 프로토콜의 JS 쪽 URL. Rust 의 peek.rs 가 `peek://localhost/…` 와 함께 받는다. */
const PEEK_ORIGIN = 'http://peek.localhost/'

/** Rust 가 보내는 이벤트 이름(src-tauri/src/preview.rs 와 같아야 한다) */
const Events = {
  show: 'preview:show',
  showError: 'preview:show-error',
  hide: 'preview:hide',
  copied: 'preview:copied'
} as const

function fail(what: string, e: unknown): void {
  console.error(`[peekaboo] ${what} 실패`, e)
}

export function createTauriHost(): Host {
  // listen() 은 비동기라 등록이 끝나기 전에 Rust 가 show 를 보내면 놓친다. 모든 리스너 등록이 끝나면
  // ready 커맨드로 알리고, Rust 는 그 뒤에만 이벤트를 보낸다. app.ts 는 모듈이 실행되는 동안 동기적으로
  // 리스너를 등록하므로 첫 등록 직후 한 틱 뒤에 보내면 전부 모인 상태다.
  const registering: Promise<unknown>[] = []
  let readyScheduled = false
  const on = <T>(event: string, cb: (payload: T) => void): void => {
    registering.push(listen<T>(event, (e) => cb(e.payload)))
    if (readyScheduled) return
    readyScheduled = true
    setTimeout(() => {
      void Promise.all(registering)
        .then(() => invoke('ready'))
        .catch((e) => fail('ready', e))
    }, 0)
  }
  const send = (cmd: string, args?: Record<string, unknown>): void => {
    invoke(cmd, args).catch((e) => fail(cmd, e))
  }

  const api: PeekabooApi = {
    onShow: (cb) => on(Events.show, cb),
    onShowError: (cb) => on(Events.showError, cb),
    onHide: (cb) => on(Events.hide, () => cb()),
    // 복사 알림은 Rust 의 복사 구현(M3)이 보낸다. 아직 보내는 쪽이 없어 호출되지 않는다.
    onCopied: (cb) => on(Events.copied, () => cb()),
    close: () => send('close'),
    textSelected: (selected) => send('text_selected', { selected }),
    // Ctrl+C 복사 요청(헬퍼가 키를 가로채 Rust 가 렌더러에 선택 텍스트를 묻는다)은 M3 에서 구현한다. 지금은 요청이 오지 않는다.
    onCopyRequest: () => {},
    listDir: (path, max) => invoke('list_dir', { path, max }),
    fileUrl: (path) => PEEK_ORIGIN + encodeURIComponent(path),
    rendered: (seq) => send('rendered', { seq }),
    fit: (seq, size: ContentSize | null) => send('fit', { seq, size }),
    nativePreview: {
      // 헬퍼 연결과 네이티브 핸들러 창은 M3 에서 구현한다. 'failed' 로 답해 플러그인이 자체 렌더러로 대체하게 한다.
      open: (): Promise<NativePreviewResult> =>
        Promise.resolve({ handle: 0, status: 'failed', message: 'Tauri 호스트는 아직 Windows 미리보기를 지원하지 않습니다' }),
      resize: () => {},
      close: () => {}
    }
  }

  return { ...api, ...createFileOps((path) => api.fileUrl(path)) }
}
