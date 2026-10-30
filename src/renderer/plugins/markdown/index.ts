import MarkdownIt from 'markdown-it'
import DOMPurify from 'dompurify'
// GitHub 의 README 렌더링과 같은 모양(라이트/다크는 OS 설정을 따른다)
import 'github-markdown-css/github-markdown.css'
import type { PreviewPlugin, RenderContext } from '../types'
import { ensureLanguage, highlightSync, resolveLanguage } from '../text/highlight'
import { headingEntries, mountToc } from '../toc'

const MAX_BYTES = 2 * 1024 * 1024

// GitHub 의 > [!NOTE] 류 알림 상자
const ALERTS: Record<string, string> = {
  NOTE: 'Note',
  TIP: 'Tip',
  IMPORTANT: 'Important',
  WARNING: 'Warning',
  CAUTION: 'Caution',
}

export function create(): PreviewPlugin {
  let off: (() => void) | undefined
  return {
    async render({ file, container, signal, host, setSubtitle }: RenderContext) {
      const res = await host.readText(file.path, MAX_BYTES)
      if (signal.aborted) return

      // GitHub 처럼 본문 속 HTML 도 표시한다. 결과는 DOMPurify 가 정리한다.
      const md = new MarkdownIt({
        html: true,
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

      // GitHub 처럼 스타일·폼 요소는 지운다(헤더를 덮거나 CSS url() 로 요청을 보내지 못하게)
      const frag = DOMPurify.sanitize(md.render(res.text), {
        RETURN_DOM_FRAGMENT: true,
        FORBID_TAGS: ['style', 'form', 'input', 'button', 'textarea', 'select'],
        FORBID_ATTR: ['style'],
      })

      // 제한: main 은 현재 파일만 서빙하므로 상대 경로 이미지는 불러올 수 없다.
      // 깨진 아이콘 대신 대체 텍스트로 바꾼다. data: 이미지만 유지한다.
      frag.querySelectorAll('img').forEach((img) => {
        if (/^data:image\//i.test(img.getAttribute('src') ?? '')) return
        const span = document.createElement('span')
        span.className = 'img-missing'
        span.textContent = img.getAttribute('alt') || '[이미지]'
        img.replaceWith(span)
      })
      taskLists(frag)
      alerts(frag)

      container.classList.add('md-view')
      const scroll = document.createElement('div')
      scroll.className = 'md-scroll'
      const article = document.createElement('article')
      article.className = 'markdown-body'
      article.append(frag)
      scroll.append(article)
      container.append(scroll)
      const unmountToc = mountToc(container, scroll, headingEntries(article), {
        target: scroll,
        viewTop: () => scroll.getBoundingClientRect().top,
      })

      // 링크는 창을 이동시키지 않는다(main 도 차단하지만 렌더러에서 먼저 막는다).
      const block = (e: Event) => {
        if ((e.target as Element | null)?.closest('a')) e.preventDefault()
      }
      container.addEventListener('click', block)
      container.addEventListener('auxclick', block)
      off = () => {
        container.removeEventListener('click', block)
        container.removeEventListener('auxclick', block)
        unmountToc?.()
      }

      setSubtitle(res.truncated ? 'Markdown · 처음 2MB만 표시' : 'Markdown')
    },
    dispose() {
      off?.()
    },
  }
}

/** "- [ ] 할 일" / "- [x] 한 일" → 체크박스(읽기 전용) */
function taskLists(root: DocumentFragment): void {
  root.querySelectorAll('li').forEach((li) => {
    // 느슨한 목록이면 <li><p>[ ] …</p></li> 형태가 된다
    const host = li.firstElementChild?.tagName === 'P' ? li.firstElementChild : li
    const first = host.firstChild
    if (first?.nodeType !== Node.TEXT_NODE) return
    const m = /^\[([ xX])\]\s/.exec(first.textContent ?? '')
    if (!m) return
    first.textContent = (first.textContent ?? '').slice(m[0].length)
    const box = document.createElement('input')
    box.type = 'checkbox'
    box.className = 'task-list-item-checkbox'
    box.disabled = true
    box.checked = m[1] !== ' '
    host.insertBefore(box, first)
    box.after(' ')
    li.classList.add('task-list-item')
    li.parentElement?.classList.add('contains-task-list')
  })
}

/** 최상위의 "> [!NOTE]" 로 시작하는 인용문 → GitHub 알림 상자(GitHub 도 중첩된 것은 바꾸지 않는다) */
function alerts(root: DocumentFragment): void {
  Array.from(root.children).forEach((bq) => {
    if (bq.tagName !== 'BLOCKQUOTE') return
    const p = bq.firstElementChild
    if (p?.tagName !== 'P' || p.firstChild?.nodeType !== Node.TEXT_NODE) return
    const text = p.firstChild.textContent ?? ''
    const m = /^\[!(\w+)\][ \t]*(\n|$)/.exec(text)
    const label = m && ALERTS[m[1].toUpperCase()]
    if (!m || !label) return

    p.firstChild.textContent = text.slice(m[0].length)
    // 표시어 바로 뒤의 줄바꿈 태그와 빈 문단은 지운다
    if (p.firstChild.textContent === '' && p.firstChild.nextSibling?.nodeName === 'BR') p.firstChild.nextSibling.remove()
    if (!p.textContent?.trim() && !p.querySelector('img, code')) p.remove()

    const div = document.createElement('div')
    div.className = `markdown-alert markdown-alert-${m[1].toLowerCase()}`
    const title = document.createElement('p')
    title.className = 'markdown-alert-title'
    title.textContent = label
    div.append(title, ...bq.childNodes)
    bq.replaceWith(div)
  })
}
