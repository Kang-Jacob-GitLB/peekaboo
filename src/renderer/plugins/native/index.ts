// Windows 미리보기 핸들러 우선 플러그인.
// Office 문서처럼 브라우저로 충실히 그리기 어려운 형식은 Windows 에 등록된 미리보기 핸들러
// (탐색기 "미리 보기 창"과 같은 것)를 헬퍼가 이 창 위에 자식 창으로 띄운다.
// 핸들러가 없거나 실패하면 fallback 으로 받은 자체 렌더러로 그린다.

import type { PreviewPlugin, RenderContext } from '../types'
import type { PixelRect } from '../../../shared/preview-api'

type PluginModule = { create: () => PreviewPlugin }

/** registry 의 load 에 그대로 넣을 수 있는 모듈 모양으로 감싼다. */
export function nativeFirst(loadFallback?: () => Promise<PluginModule>): PluginModule {
  return { create: () => new NativeFirstPlugin(loadFallback) }
}

class NativeFirstPlugin implements PreviewPlugin {
  private handle: number | null = null
  private observer: ResizeObserver | null = null
  private fallback: PreviewPlugin | null = null
  private host: RenderContext['host'] | null = null

  constructor(private readonly loadFallback?: () => Promise<PluginModule>) {}

  async render(ctx: RenderContext): Promise<void> {
    const { container, file, host, signal } = ctx
    this.host = host
    container.classList.add('native-host')
    // 핸들러 창이 덮기 전까지 보이는 안내(Office 핸들러는 첫 표시에 1~2초 걸린다)
    const wait = document.createElement('div')
    wait.className = 'native-wait'
    wait.innerHTML = '<span class="spinner"></span><span>Windows 미리보기를 불러오는 중…</span>'
    container.append(wait)

    const result = await host.nativePreview.open(file.path, pixelRect(container))
    if (signal.aborted) {
      host.nativePreview.close(result.handle)
      return
    }

    if (result.status === 'shown') {
      this.handle = result.handle
      this.observer = new ResizeObserver(() => {
        if (this.handle !== null) host.nativePreview.resize(this.handle, pixelRect(container))
      })
      this.observer.observe(container)
      return
    }

    // 핸들러 없음/실패 → 자체 렌더러
    container.classList.remove('native-host')
    container.replaceChildren()
    if (!this.loadFallback) {
      throw new Error(
        result.status === 'unavailable'
          ? '이 형식을 미리 볼 수 있는 Windows 미리보기 프로그램이 설치돼 있지 않습니다.'
          : `Windows 미리보기를 표시하지 못했습니다(${result.message ?? '알 수 없는 오류'}).`
      )
    }
    const mod = await this.loadFallback()
    if (signal.aborted) return
    this.fallback = mod.create()
    await this.fallback.render(ctx)
  }

  dispose(): void {
    this.observer?.disconnect()
    this.observer = null
    if (this.handle !== null) this.host?.nativePreview.close(this.handle)
    this.handle = null
    this.fallback?.dispose?.()
    this.fallback = null
  }
}

/** 요소 영역을 창 클라이언트 기준 물리 픽셀로. 프레임 없는 창이라 클라이언트 원점 = 뷰포트 원점. */
function pixelRect(el: HTMLElement): PixelRect {
  const r = el.getBoundingClientRect()
  const k = window.devicePixelRatio
  return { x: Math.round(r.left * k), y: Math.round(r.top * k), width: Math.round(r.width * k), height: Math.round(r.height * k) }
}
