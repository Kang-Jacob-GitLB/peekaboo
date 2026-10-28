// main <-> renderer(미리보기 창) 계약. preload 가 window.peekaboo 로 노출한다.

export interface FileInfo {
  path: string
  name: string
  /** 소문자, 점 제외 ("png"). 확장자 없으면 "" */
  ext: string
  size: number
  isDirectory: boolean
  mtimeMs: number
}

export interface ShowPayload {
  /** 화면 표시 순번. 렌더러는 늦게 끝난 이전 렌더 결과를 버리는 데 쓴다. */
  seq: number
  file: FileInfo
  /** 탐색기에서 선택된 항목 수(포커스 항목 하나만 미리보기) */
  selectionCount: number
}

export interface ShowError {
  seq: number
  path: string
  message: string
}

export interface TextResult {
  text: string
  /** maxBytes 에서 잘렸으면 true */
  truncated: boolean
  /** 감지한 인코딩: "utf-8" | "utf-16le" | "utf-16be" | "euc-kr" */
  encoding: string
}

export interface DirEntry {
  name: string
  isDirectory: boolean
  size: number
  mtimeMs: number
}

export interface PeekabooApi {
  /** 새 파일 표시 요청 */
  onShow(cb: (payload: ShowPayload) => void): void
  /** 대상 파일을 열 수 없을 때(삭제됨, 권한 없음 등) */
  onShowError(cb: (err: ShowError) => void): void
  /** 창이 숨겨지기 직전 — 재생 중인 미디어 정지 등 정리용 */
  onHide(cb: () => void): void
  /** 현재 표시 중인 파일만 읽을 수 있다(main 이 검증). */
  readText(path: string, maxBytes: number): Promise<TextResult>
  /** 현재 표시 중인 폴더의 항목(최대 max 개) */
  listDir(path: string, max: number): Promise<{ entries: DirEntry[]; truncated: boolean }>
  /** <img>/<video> 등에 쓸 수 있는 peek:// URL. 현재 표시 중인 파일만 서빙된다. */
  fileUrl(path: string): string
  /** 렌더 완료 신호(첫 표시 지연 측정용) */
  rendered(seq: number): void
}
