// 미리보기 렌더러 플러그인 공통 인터페이스.
// 새 포맷 추가 = 이 인터페이스를 구현한 모듈 하나 + registry.ts 에 항목 한 줄.

export interface FileInfo {
  path: string
  name: string
  ext: string // 소문자, 점 제외 ("png")
  size: number
  isDirectory: boolean
  mtimeMs: number
}

export interface RenderContext {
  file: FileInfo
  container: HTMLElement
  /** 다른 파일로 넘어가거나 창이 닫히면 abort 된다. 비동기 작업은 이걸 확인할 것. */
  signal: AbortSignal
  host: PreviewHostApi
}

/** 렌더러가 main 프로세스에 요청할 수 있는 기능(preload 가 노출) */
export interface PreviewHostApi {
  readText(path: string, maxBytes: number): Promise<{ text: string; truncated: boolean; encoding: string }>
  fileUrl(path: string): string
}

export interface PreviewPlugin {
  id: string
  render(ctx: RenderContext): Promise<void>
  dispose?(): void
}

export interface PluginEntry {
  id: string
  /** 처리 가능한 확장자(소문자, 점 제외). '*' 는 폴백. */
  extensions: string[]
  matchDirectory?: boolean
  /** 실제 사용될 때만 import 되도록 지연 로딩 */
  load: () => Promise<{ default: PreviewPlugin }>
}
