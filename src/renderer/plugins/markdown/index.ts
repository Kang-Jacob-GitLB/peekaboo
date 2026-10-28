import MarkdownIt from 'markdown-it'
import DOMPurify from 'dompurify'
import type { PreviewPlugin, RenderContext } from '../types'
import { ensureLanguage, highlightSync, resolveLanguage } from '../text/highlight'

const MAX_BYTES = 2 * 1024 * 1024

export function create(): PreviewPlugin {
  let off: (() => void) | undefined
  return {
    async render({ file, container, signal, host, setSubtitle }: RenderContext) {
      const res = await host.readText(file.path, MAX_BYTES)
      if (signal.aborted) return

      const md = new MarkdownIt({
        html: false,
        linkify: true,
        highlight: (code, lang) => {
          const l = lang ? resolveLanguage(lang.trim().split(/\s+/)[0]) : undefined
          return l ? `<pre class="hljs"><code>${highlightSync(code, l)}</code></pre>` : ''
        },
      })

      // 코드펜스에 쓰인 언어만 미리 로드(하이라이트 콜백은 동기여야 하므로).
      const langs = new Set<string>()
      for (const t of md.parse(res.text, {})) {
        if (t.type === 'fence' && t.info.trim()) langs.add(t.info.trim().split(/\s+/)[0])
      }
      await Promise.all([...langs].map((l) => ensureLanguage(l)))
      if (signal.aborted) return

      const frag = DOMPurify.sanitize(md.render(res.text), { RETURN_DOM_FRAGMENT: true })

      // 제한: main 은 현재 파일만 서빙하므로 상대 경로 이미지는 불러올 수 없다.
      // 깨진 아이콘 대신 대체 텍스트로 바꾼다. data: 이미지만 유지한다.
      frag.querySelectorAll('img').forEach((img) => {
        if (/^data:image\//i.test(img.getAttribute('src') ?? '')) return
        const span = document.createElement('span')
        span.className = 'img-missing'
        span.textContent = img.getAttribute('alt') || '[이미지]'
        img.replaceWith(span)
      })

      container.classList.add('md-view')
      const article = document.createElement('article')
      article.className = 'markdown-body'
      article.append(frag)
      container.append(article)

      // 링크는 창을 이동시키지 않는다(main 도 차단하지만 렌더러에서 먼저 막는다).
      const block = (e: Event) => {
        if ((e.target as Element | null)?.closest('a')) e.preventDefault()
      }
      container.addEventListener('click', block)
      container.addEventListener('auxclick', block)
      off = () => {
        container.removeEventListener('click', block)
        container.removeEventListener('auxclick', block)
      }

      setSubtitle(res.truncated ? 'Markdown · 처음 2MB만 표시' : 'Markdown')
    },
    dispose() {
      off?.()
    },
  }
}
