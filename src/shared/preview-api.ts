// main <-> renderer(미리보기 창) 계약. preload 가 window.peekaboo 로 노출한다.

import type { NativePreviewStatus, PixelRect } from './ipc-types'

export type { NativePreviewStatus, PixelRect }

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

/** 미리보기 창을 맞출 내용 크기(CSS px). 창은 이 비율을 유지하며 화면에 들어가게 줄인다. */
export interface ContentSize {
  width: number
  height: number
  /** true 면 화면 한도까지 키운다(문서처럼 비율만 의미 있는 경우). false 면 원래 크기보다 키우지 않는다. */
  grow?: boolean
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

export interface ArchiveEntry {
  /** 압축 파일 안의 경로("/" 구분). 디렉터리 항목은 끝에 "/" 가 붙어 있다. */
  path: string
  isDirectory: boolean
  /** 원본(압축 해제 후) 크기 */
  size: number
  compressedSize: number
  /** 수정 시각(ms). 알 수 없으면 0 */
  mtimeMs: number
  encrypted: boolean
  /** zip 압축 방식 번호(0=저장, 8=deflate ...) */
  method: number
}

export interface ArchiveListing {
  entries: ArchiveEntry[]
  /** 전체 항목 수(zip 은 EOCD 기록값, tar 는 읽은 수) */
  totalEntries: number
  /** 항목이 max 를 넘어 entries 가 일부뿐이면 true */
  truncated: boolean
  /** entries 에 담긴 파일의 원본 크기 합(truncated 면 일부만 합산) */
  totalSize: number
  compressedSize: number
  /** 하나라도 암호화돼 있으면 true */
  encrypted: boolean
  comment?: string
  /** totalEntries 가 정확한 값이 아니라 하한(tar 처럼 색인이 없는 형식을 끝까지 읽지 못함) */
  totalIsLowerBound?: boolean
  /** 목록 정보가 너무 커서(zip 중앙 디렉터리 64MB, tar.gz 해제 256MB) 앞부분만 읽었음 */
  scanLimited?: boolean
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
  /** 현재 표시 중인 zip 계열 압축 파일의 항목 목록(압축 해제 없이 중앙 디렉터리만 읽는다) */
  listArchive(path: string, max: number): Promise<ArchiveListing>
  /** <img>/<video> 등에 쓸 수 있는 peek:// URL. 현재 표시 중인 파일만 서빙된다. */
  fileUrl(path: string): string
  /** 렌더 완료 신호(첫 표시 지연 측정용) */
  rendered(seq: number): void
  /** 창 크기를 내용(size)에 맞춘다. null 이면 기본 크기로 되돌린다. */
  fit(seq: number, size: ContentSize | null): void
  /**
   * Windows 에 등록된 미리보기 핸들러(Office 등)를 rect(이 창 클라이언트 영역, 물리 픽셀) 위치에 띄운다.
   * 핸들러 창은 HTML 위에 겹쳐지는 별도 네이티브 창이다. 한 번에 하나만 열리며 새로 열면 이전 것은 닫힌다.
   * 결과가 'shown' 이 아니면 플러그인이 자체 렌더러로 대체해야 한다.
   */
  nativePreview: {
    open(path: string, rect: PixelRect): Promise<NativePreviewResult>
    resize(handle: number, rect: PixelRect): void
    close(handle: number): void
  }
}

export interface NativePreviewResult {
  /** resize/close 에 쓰는 식별자 */
  handle: number
  status: NativePreviewStatus
  message?: string
}
