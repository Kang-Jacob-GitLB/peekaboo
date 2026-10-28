// 미리보기 렌더러 플러그인 공통 인터페이스.
// 새 포맷 추가 = 이 인터페이스를 구현한 모듈 하나 + registry.ts 에 항목 한 줄.

import type { FileInfo, PeekabooApi } from '../../shared/preview-api'

export type { FileInfo }

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
  /** 실제 사용될 때만 import 되도록 지연 로딩. 호출할 때마다 새 인스턴스를 만든다. */
  load: () => Promise<{ create: () => PreviewPlugin }>
}
