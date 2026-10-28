// Helper <-> Electron 프로토콜 (stdin/stdout JSON Lines). 스펙 문서: docs/ipc.md
// C# 쪽 대응 타입: helper/Ipc/Messages.cs — 필드를 바꾸면 양쪽을 함께 고칠 것.

export const PROTOCOL_VERSION = 1

export type KeyName = 'space' | 'esc' | 'enter' | 'up' | 'down' | 'left' | 'right'

export type HelperErrorCode = 'ELEVATED' | 'NO_VIEW' | 'COM_FAIL' | 'BAD_MESSAGE' | 'INTERNAL'

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

// ---- Electron -> Helper ----
export type HostMessage =
  | { v: 1; type: 'config'; enabled: boolean }
  | { v: 1; type: 'state'; previewOpen: boolean }
  | { v: 1; type: 'getSelection'; id: number }
  | { v: 1; type: 'ping'; id: number }
  | { v: 1; type: 'shutdown' }
