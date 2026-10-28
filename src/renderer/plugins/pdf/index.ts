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
}

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
      loadingTask = pdfjs.getDocument({ data: new Uint8Array(data) })
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

      const renderPage = async (s: Slot) => {
        if (s.state !== 0) return
        s.state = 1
        try {
          const page = await pdf.getPage(s.num)
          if (signal.aborted || s.state !== 1) return
          const scale = fitScale()
          const css = page.getViewport({ scale })
          const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR)
          const vp = page.getViewport({ scale: scale * dpr })
          s.canvas.width = Math.floor(vp.width)
          s.canvas.height = Math.floor(vp.height)
          s.el.style.width = `${Math.floor(css.width)}px`
          s.el.style.height = `${Math.floor(css.height)}px`
          s.task = page.render({ canvas: s.canvas, viewport: vp })
          await s.task.promise
          s.state = 2
        } catch {
          // 취소(리사이즈) 또는 개별 페이지 실패 — 다시 보이면 재시도한다.
          s.state = 0
        }
      }

      observer = new IntersectionObserver(
        (entries) => {
          for (const e of entries) {
            const s = slotOf.get(e.target)
            if (e.isIntersecting && s) void renderPage(s)
          }
        },
        // 화면 밖 한 화면 정도는 미리 그려 둔다.
        { root: container, rootMargin: '100% 0px' }
      )

      const slots: Slot[] = []
      for (let i = 1; i <= pdf.numPages; i++) {
        const el = document.createElement('div')
        el.className = 'pdf-page'
        const canvas = document.createElement('canvas')
        el.append(canvas)
        const s: Slot = { num: i, el, canvas, state: 0 }
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
          s.task?.cancel()
          s.state = 0
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
