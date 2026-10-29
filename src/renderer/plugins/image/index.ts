import type { PreviewPlugin, RenderContext } from '../types'

// <img> 로 로드하므로 SVG 안의 스크립트는 실행되지 않는다.
export function create(): PreviewPlugin {
  return {
    async render({ file, container, signal, host, setSubtitle, fitContent }: RenderContext) {
      container.classList.add('image-view')
      const img = document.createElement('img')
      img.alt = file.name
      img.draggable = false
      container.append(img)

      await new Promise<void>((resolve, reject) => {
        img.onload = () => resolve()
        img.onerror = () => reject(new Error('이미지를 불러올 수 없습니다.'))
        signal.addEventListener('abort', () => resolve(), { once: true })
        img.src = host.fileUrl(file.path)
      })
      if (signal.aborted) return
      const w = img.naturalWidth
      const h = img.naturalHeight
      // 작은 이미지는 확대하지 않고(최대 100%), 큰 이미지만 맞춰 줄인다.
      if (w && h) {
        img.style.maxWidth = `min(100%, ${w}px)`
        img.style.maxHeight = `min(100%, ${h}px)`
        setSubtitle(`${file.ext.toUpperCase()} · ${w}×${h}`)
        // .image-view 의 padding 12px 를 더한다
        fitContent({ width: w + 24, height: h + 24 })
      }
      try {
        await img.decode()
      } catch {
        /* 일부 SVG 는 decode 가 거부되지만 표시는 된다 */
      }
    },
  }
}
