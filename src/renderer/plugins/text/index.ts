import type { PreviewPlugin, RenderContext } from '../types'
import { ensureLanguage, highlightSync, escapeHtml } from './highlight'

const MAX_BYTES = 512 * 1024
const HIGHLIGHT_LIMIT = 200 * 1024

export function create(): PreviewPlugin {
  return {
    async render({ file, container, signal, host, setSubtitle }: RenderContext) {
      const res = await host.readText(file.path, MAX_BYTES)
      if (signal.aborted) return

      const text = res.text.replace(/\r\n?/g, '\n')
      const doHighlight = text.length <= HIGHLIGHT_LIMIT
      let lang: string | undefined
      if (doHighlight) {
        lang = (await ensureLanguage(file.ext)) ?? (await ensureLanguage(file.name))
        if (signal.aborted) return
      }

      const lines = text.split('\n')
      if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop()

      container.classList.add('text-view')
      if (res.truncated) {
        const n = document.createElement('div')
        n.className = 'notice'
        n.setAttribute('role', 'status')
        n.textContent = '처음 512KB만 표시'
        container.append(n)
      }

      const scroll = document.createElement('div')
      scroll.className = 'code-scroll'
      const gutter = document.createElement('pre')
      gutter.className = 'gutter'
      gutter.setAttribute('aria-hidden', 'true')
      gutter.textContent = Array.from({ length: lines.length }, (_, i) => i + 1).join('\n')
      const code = document.createElement('pre')
      code.className = 'code hljs'
      const body = lines.join('\n')
      code.innerHTML = lang ? highlightSync(body, lang) : escapeHtml(body)
      scroll.append(gutter, code)
      container.append(scroll)

      const enc = res.encoding.toLowerCase()
      const parts = [lang ?? (file.ext || file.name).toUpperCase()]
      if (enc !== 'utf-8') parts.push(enc.toUpperCase())
      setSubtitle(parts.join(' · '))
    },
  }
}
