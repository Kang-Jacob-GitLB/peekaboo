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

export function create(): PreviewPlugin {
  return {
    async render({ file, container }: RenderContext) {
      renderFallback(container, file)
    },
  }
}
