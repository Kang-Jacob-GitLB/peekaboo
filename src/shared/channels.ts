// Electron IPC 채널 이름과 peek:// URL 규칙 (main / preload 공용)

export const Channels = {
  show: 'preview:show',
  showError: 'preview:show-error',
  hide: 'preview:hide',
  readText: 'preview:read-text',
  listDir: 'preview:list-dir',
  rendered: 'preview:rendered'
} as const

export const PEEK_SCHEME = 'peek'

/** 로컬 경로 → peek://file/<encodeURIComponent(path)> */
export function toPeekUrl(path: string): string {
  return `${PEEK_SCHEME}://file/${encodeURIComponent(path)}`
}

export function fromPeekUrl(url: string): string | null {
  const prefix = `${PEEK_SCHEME}://file/`
  if (!url.startsWith(prefix)) return null
  try {
    return decodeURIComponent(url.slice(prefix.length).split(/[?#]/)[0])
  } catch {
    return null
  }
}
