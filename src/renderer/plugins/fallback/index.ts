import type { PreviewPlugin, RenderContext } from '../types'
import { formatDate, formatSize } from '../format'

const ICON = `<svg viewBox="0 0 64 64" width="96" height="96" aria-hidden="true" focusable="false">
<path d="M14 6h25l13 13v37a2 2 0 0 1-2 2H14a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2z" fill="var(--icon-body)" stroke="var(--icon-edge)" stroke-width="2" stroke-linejoin="round"/>
<path d="M39 6v13h13" fill="var(--icon-fold)" stroke="var(--icon-edge)" stroke-width="2" stroke-linejoin="round"/>
</svg>`
const FOLDER = `<svg viewBox="0 0 64 64" width="96" height="96" aria-hidden="true" focusable="false">
<path d="M6 14a3 3 0 0 1 3-3h15l6 6h25a3 3 0 0 1 3 3v31a3 3 0 0 1-3 3H9a3 3 0 0 1-3-3z" fill="var(--icon-folder)" stroke="var(--icon-edge)" stroke-width="2" stroke-linejoin="round"/>
</svg>`

/** app.ts 가 플러그인 오류 시에도 재사용한다. */
export function renderFallback(
  container: HTMLElement,
  info: { name: string; size?: number; mtimeMs?: number; isDirectory?: boolean },
  message = '미리보기를 지원하지 않는 형식입니다',
): void {
  container.classList.add('fallback-view')
  const icon = document.createElement('div')
  icon.className = 'fb-icon'
  icon.innerHTML = info.isDirectory ? FOLDER : ICON
  const name = document.createElement('div')
  name.className = 'fb-name'
  name.textContent = info.name
  const meta = document.createElement('div')
  meta.className = 'fb-meta'
  const bits: string[] = []
  if (!info.isDirectory && info.size != null) bits.push(formatSize(info.size))
  if (info.mtimeMs != null) bits.push(`수정: ${formatDate(info.mtimeMs)}`)
  meta.textContent = bits.join(' · ')
  const msg = document.createElement('div')
  msg.className = 'fb-msg'
  msg.textContent = message
  container.append(icon, name, meta, msg)
}

const SNIFF_BYTES = 4096

/**
 * 확장자로 형식을 정하지 못한 파일의 앞부분이 텍스트인지 추정한다.
 * readText 가 UTF-8 → EUC-KR 순으로 디코딩하므로, NUL 이 있거나 제어 문자·깨진 문자(U+FFFD)가 많으면 바이너리로 본다.
 */
async function looksLikeText(host: RenderContext['host'], path: string): Promise<boolean> {
  let text: string
  try {
    text = (await host.readText(path, SNIFF_BYTES)).text
  } catch {
    return false
  }
  if (text.length === 0) return false
  let bad = 0
  for (const ch of text) {
    const c = ch.codePointAt(0)!
    if (c === 0) return false
    // 탭·줄바꿈·폼피드·ESC(ANSI 색 코드) 외의 C0 제어 문자, DEL, 디코딩 실패 문자
    if ((c < 0x20 && c !== 0x09 && c !== 0x0a && c !== 0x0d && c !== 0x0c && c !== 0x1b) || c === 0x7f || c === 0xfffd) bad++
  }
  return bad * 100 <= text.length
}

export function create(): PreviewPlugin {
  // 내용이 텍스트라 텍스트 플러그인에 넘긴 경우(확장자만 특이한 설정·목록 파일 등)
  let delegate: PreviewPlugin | undefined
  return {
    async render(ctx: RenderContext) {
      const { file, container, signal, host } = ctx
      if (!file.isDirectory && file.size > 0 && (await looksLikeText(host, file.path))) {
        if (signal.aborted) return
        const text = (await import('../text')).create()
        if (signal.aborted) return
        delegate = text
        return text.render(ctx)
      }
      if (signal.aborted) return
      renderFallback(container, file)
    },
    dispose() {
      delegate?.dispose?.()
    },
  }
}
