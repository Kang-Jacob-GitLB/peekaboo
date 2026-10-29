// Helper <-> Electron 프로토콜 (stdin/stdout JSON Lines). 스펙 문서: docs/ipc.md
// C# 쪽 대응 타입: helper/Ipc/Messages.cs — 필드를 바꾸면 양쪽을 함께 고칠 것.

export const PROTOCOL_VERSION = 1

export type KeyName = 'space' | 'esc' | 'enter' | 'up' | 'down' | 'left' | 'right'

export type HelperErrorCode = 'ELEVATED' | 'NO_VIEW' | 'COM_FAIL' | 'BAD_MESSAGE' | 'INTERNAL'

/** 미리보기 창 클라이언트 영역 기준 물리 픽셀 */
export interface PixelRect {
  x: number
  y: number
  width: number
  height: number
}

/**
 * Windows 미리보기 핸들러 결과.
 * shown = 표시됨, unavailable = 등록된 핸들러 없음, failed = 생성/초기화/표시 실패
 */
export type NativePreviewStatus = 'shown' | 'unavailable' | 'failed'

// ---- Helper -> Electron ----
export type HelperMessage =
  | { v: 1; type: 'ready'; version: string; pid: number }
  | { v: 1; type: 'key'; key: KeyName; consumed: boolean }
  | {
      v: 1
      type: 'selection'
      id?: number
      paths: string[]
      focused: string | null
      source: 'explorer' | 'desktop'
      hwnd: number
    }
  | { v: 1; type: 'pong'; id: number }
  | { v: 1; type: 'error'; id?: number; code: HelperErrorCode; message: string }
  | { v: 1; type: 'nativePreview'; id: number; status: NativePreviewStatus; message?: string }
  /** 포그라운드가 셸(탐색기·바탕화면·작업 표시줄·시작 메뉴 등) 밖의 창으로 넘어감. pid = 그 창의 프로세스 */
  | { v: 1; type: 'foreground'; pid: number }

// ---- Electron -> Helper ----
export type HostMessage =
  | { v: 1; type: 'config'; enabled: boolean }
  | { v: 1; type: 'state'; previewOpen: boolean }
  | { v: 1; type: 'getSelection'; id: number }
  | { v: 1; type: 'ping'; id: number }
  | { v: 1; type: 'shutdown' }
  | { v: 1; type: 'nativePreview.open'; id: number; path: string; hwnd: number; rect: PixelRect; dark: boolean }
  | { v: 1; type: 'nativePreview.resize'; id: number; rect: PixelRect }
  | { v: 1; type: 'nativePreview.close'; id: number }
