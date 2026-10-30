import type { PreviewPlugin, RenderContext } from '../types'
import { headingEntries, mountToc } from '../toc'

const MAX_BYTES = 2 * 1024 * 1024

/**
 * HTML 을 브라우저처럼 렌더링해 보여 준다.
 * - sandbox 에 allow-scripts 가 없어 스크립트·폼·팝업은 동작하지 않는다.
 *   allow-same-origin 은 부모가 링크 클릭을 막으려고 문서에 접근하는 데만 쓴다(스크립트가 없으니 안전).
 * - srcdoc 문서는 앱의 CSP 를 물려받으므로 외부(http) 리소스는 불러오지 않는다.
 *   상대 경로는 HTML 파일 위치가 아니라 앱 페이지 기준으로 풀리므로, 옆 파일의 이미지·CSS 는 표시되지 않는다.
 * - 다른 문서로의 이동은 main 의 will-frame-navigate 가, UNC 경로 요청은 blockRemoteFileRequests 가 막는다.
 */
export function create(): PreviewPlugin {
  let off: (() => void) | undefined
  let unmountToc: (() => void) | null = null
  return {
    async render({ file, container, signal, host, setSubtitle }: RenderContext) {
      const res = await host.readText(file.path, MAX_BYTES)
      if (signal.aborted) return

      container.classList.add('html-view')
      const frame = document.createElement('iframe')
      frame.setAttribute('sandbox', 'allow-same-origin')
      frame.setAttribute('referrerpolicy', 'no-referrer')
      frame.title = file.name
      frame.srcdoc = withoutRefresh(res.text)

      // 링크는 문서 안 이동(#id)만 허용한다
      const block = (e: Event) => {
        const a = (e.target as Element | null)?.closest?.('a')
        if (a && !a.getAttribute('href')?.startsWith('#')) e.preventDefault()
      }
      frame.addEventListener('load', () => {
        const doc = frame.contentDocument
        if (!doc) return
        doc.addEventListener('click', block)
        doc.addEventListener('auxclick', block)
        // 제목 목차. 문서 스크롤은 iframe 안의 window 에서 일어난다.
        if (signal.aborted || unmountToc || !frame.contentWindow) return
        unmountToc = mountToc(container, frame, headingEntries(doc), { target: frame.contentWindow, viewTop: () => 0 })
      })
      off = () => frame.remove()
      container.append(frame)

      setSubtitle(res.truncated ? 'HTML · 처음 2MB만 표시' : 'HTML')
    },
    dispose() {
      unmountToc?.()
      off?.()
    },
  }
}

/** <meta http-equiv="refresh"> 로 다른 곳으로 이동하지 않게 지운다 */
function withoutRefresh(html: string): string {
  return html.replace(/<meta\b[^>]*http-equiv\s*=\s*["']?refresh[^>]*>/gi, '')
}
