import * as pdfjs from 'pdfjs-dist'
import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url'
import type { PDFDocumentLoadingTask, PDFDocumentProxy, RenderTask } from 'pdfjs-dist'
import type { PreviewPlugin, RenderContext } from '../types'
import { readBinary } from '../read-binary'

// 워커는 번들에 포함된 로컬 파일을 쓴다(CSP worker-src 'self').
pdfjs.GlobalWorkerOptions.workerSrc = workerUrl

const PAD = 16
const MAX_DPR = 2

interface Slot {
  num: number
  el: HTMLElement
  canvas: HTMLCanvasElement
  /** 0 = 미렌더, 1 = 진행 중, 2 = 완료 */
  state: 0 | 1 | 2
  task?: RenderTask
  /** 렌더/비우기마다 올라가는 세대 번호. 오래된 비동기 렌더는 번호가 달라 결과를 버린다. */
  gen: number
}

// CMap·표준 폰트·wasm 은 렌더러 출력의 pdfjs/ 아래에 같이 싣는다(electron.vite.config.ts). dev/file:// 모두 문서 기준 상대 경로.
const assetUrl = (dir: string) => new URL(`./pdfjs/${dir}/`, document.baseURI).href

export function create(): PreviewPlugin {
  let loadingTask: PDFDocumentLoadingTask | undefined
  let observer: IntersectionObserver | undefined
  let resizeObs: ResizeObserver | undefined
  return {
    async render(ctx: RenderContext) {
      const { container, signal, setSubtitle } = ctx
      const data = await readBinary(ctx)
      if (signal.aborted) return

      // pdfjs-dist 6.x 는 eval 경로가 제거돼 unsafe-eval 없이 동작한다(isEvalSupported 옵션 없음).
      loadingTask = pdfjs.getDocument({
        data: new Uint8Array(data),
        cMapUrl: assetUrl('cmaps'),
        cMapPacked: true,
        standardFontDataUrl: assetUrl('standard_fonts'),
        wasmUrl: assetUrl('wasm'),
      })
      // 암호/손상 PDF 는 여기서 reject → app.ts 가 폴백으로 넘긴다.
      const pdf: PDFDocumentProxy = await loadingTask.promise
      if (signal.aborted) return

      container.classList.add('pdf-view')
      const pages = document.createElement('div')
      pages.className = 'pdf-pages'
      container.append(pages)

      const base = (await pdf.getPage(1)).getViewport({ scale: 1 })
      const slotOf = new Map<Element, Slot>()
      if (signal.aborted) return

      // 창 폭에 맞춘 배율. 안 그린 페이지는 1쪽 비율로 자리만 잡고, 그릴 때 실제 크기로 보정한다.
      const fitScale = () => Math.max(0.2, (container.clientWidth - PAD * 2) / base.width)
      const placeholder = (s: Slot) => {
        const scale = fitScale()
        s.el.style.width = `${Math.floor(base.width * scale)}px`
        s.el.style.height = `${Math.floor(base.height * scale)}px`
      }

      // 화면 밖으로 나간 페이지의 렌더를 취소하고 캔버스 메모리를 반납한다.
      const evict = (s: Slot) => {
        s.gen++
        s.task?.cancel()
        s.task = undefined
        s.canvas.width = 0
        s.canvas.height = 0
        s.state = 0
      }

      const renderPage = async (s: Slot) => {
        if (s.state !== 0) return
        s.state = 1
        const gen = ++s.gen
        try {
          const page = await pdf.getPage(s.num)
          if (signal.aborted || gen !== s.gen) return
          const scale = fitScale()
          const css = page.getViewport({ scale })
          const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR)
          const vp = page.getViewport({ scale: scale * dpr })
          s.canvas.width = Math.floor(vp.width)
          s.canvas.height = Math.floor(vp.height)
          s.el.style.width = `${Math.floor(css.width)}px`
          s.el.style.height = `${Math.floor(css.height)}px`
          const task = page.render({ canvas: s.canvas, viewport: vp })
          s.task = task
          await task.promise
          if (gen === s.gen) s.state = 2
        } catch {
          // 취소(화면 밖/리사이즈)는 evict 가 이미 정리했다. 개별 페이지 실패만 되돌려 다시 보이면 재시도한다.
          if (gen === s.gen) evict(s)
        }
      }

      observer = new IntersectionObserver(
        (entries) => {
          for (const e of entries) {
            const s = slotOf.get(e.target)
            if (!s) continue
            if (e.isIntersecting) void renderPage(s)
            else if (s.state !== 0) evict(s)
          }
        },
        // 화면 밖 한 화면 정도는 미리 그려 두고, 그 바깥으로 벗어나면 비운다.
        { root: container, rootMargin: '100% 0px' }
      )

      const slots: Slot[] = []
      for (let i = 1; i <= pdf.numPages; i++) {
        const el = document.createElement('div')
        el.className = 'pdf-page'
        const canvas = document.createElement('canvas')
        el.append(canvas)
        const s: Slot = { num: i, el, canvas, state: 0, gen: 0 }
        placeholder(s)
        slots.push(s)
        slotOf.set(el, s)
        pages.append(el)
        observer.observe(el)
      }

      // 창 폭이 바뀌면 그린 페이지를 비우고, 다시 관찰해 보이는 쪽만 즉시 재렌더한다.
      let lastW = container.clientWidth
      resizeObs = new ResizeObserver(() => {
        const w = container.clientWidth
        if (Math.abs(w - lastW) < 2) return
        lastW = w
        for (const s of slots) {
          evict(s)
          placeholder(s)
          observer?.unobserve(s.el)
          observer?.observe(s.el)
        }
      })
      resizeObs.observe(container)

      setSubtitle(`PDF · ${pdf.numPages}쪽`)
    },
    dispose() {
      observer?.disconnect()
      resizeObs?.disconnect()
      void loadingTask?.destroy()
    },
  }
}
