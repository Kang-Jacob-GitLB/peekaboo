import mammoth from 'mammoth'
import DOMPurify from 'dompurify'
import type { PreviewPlugin, RenderContext } from '../types'
import { readBinary } from '../read-binary'

export function create(): PreviewPlugin {
  let off: (() => void) | undefined
  return {
    async render(ctx: RenderContext) {
      const { container, signal, setSubtitle } = ctx
      const arrayBuffer = await readBinary(ctx)
      if (signal.aborted) return

      // 이미지는 data: URI 로 인라인된다(mammoth 기본 동작).
      const { value } = await mammoth.convertToHtml({ arrayBuffer })
      if (signal.aborted) return

      // data:image/ 만 허용 — DOMPurify 기본값도 img 의 data: URI 는 허용한다.
      const frag = DOMPurify.sanitize(value, { RETURN_DOM_FRAGMENT: true })
      frag.querySelectorAll('img').forEach((img) => {
        if (/^data:image\//i.test(img.getAttribute('src') ?? '')) return
        img.remove()
      })

      container.classList.add('md-view')
      const article = document.createElement('article')
      article.className = 'markdown-body docx-body'
      article.append(frag)
      if (!article.hasChildNodes()) {
        article.textContent = '(내용이 없는 문서입니다)'
        article.classList.add('docx-empty')
      }
      container.append(article)

      // 링크는 창을 이동시키지 않는다.
      const block = (e: Event) => {
        if ((e.target as Element | null)?.closest('a')) e.preventDefault()
      }
      container.addEventListener('click', block)
      container.addEventListener('auxclick', block)
      off = () => {
        container.removeEventListener('click', block)
        container.removeEventListener('auxclick', block)
      }

      setSubtitle('DOCX')
    },
    dispose() {
      off?.()
    },
  }
}
