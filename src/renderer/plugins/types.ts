// 미리보기 렌더러 플러그인 공통 인터페이스.
// 새 포맷 추가 = 이 인터페이스를 구현한 모듈 하나 + registry.ts 에 항목 한 줄.

import type { ContentSize, FileInfo, PeekabooApi } from '../../shared/preview-api'

export type { ContentSize, FileInfo }

export interface RenderContext {
  file: FileInfo
  /** 플러그인이 마음대로 채워도 되는 빈 요소 */
  container: HTMLElement
  /** 다른 파일로 넘어가거나 창이 닫히면 abort 된다. 비동기 작업 후에는 signal.aborted 를 확인할 것. */
  signal: AbortSignal
  host: PeekabooApi
  /**
   * 헤더 오른쪽 형식 힌트를 교체한다(예: "PNG · 1920×1080").
   * 호출하지 않으면 앱이 확장자 대문자(예: "PNG")를 기본값으로 표시한다.
   */
  setSubtitle(text: string): void
  /**
   * 창을 내용 크기·비율에 맞춘다(사진·영상의 원래 크기, PDF 페이지 등).
   * 호출하지 않으면 registry 의 aspect, 그것도 없으면 기본 크기가 된다.
   */
  fitContent(size: ContentSize): void
}

export interface PreviewPlugin {
  render(ctx: RenderContext): Promise<void>
  /** 다음 파일로 넘어가기 전에 호출 — 미디어 정지, 리스너 해제 등 */
  dispose?(): void
}

export interface PluginEntry {
  id: string
  /** 처리 가능한 확장자(소문자, 점 제외). '*' 는 폴백. */
  extensions: string[]
  matchDirectory?: boolean
  /** 페이지 비율이 정해진 문서(가로:세로). 플러그인을 불러오기 전에 창을 이 비율로 맞춘다. */
  aspect?: [number, number]
  /** 실제 사용될 때만 import 되도록 지연 로딩. 호출할 때마다 새 인스턴스를 만든다. */
  load: () => Promise<{ create: () => PreviewPlugin }>
}
