// 파일 해석(텍스트 디코딩, 압축 목록). 예전에는 main 이 했지만 신뢰하지 않는 파일의 바이트를
// 특권 프로세스가 아니라 샌드박스된 렌더러가 해석하도록 옮겼다. 바이트는 range-reader 로만 가져온다.

import type { ArchiveListing, TextResult } from '../../../shared/preview-api'
import { listArchive } from './archive.ts'
import { createRangeReader, type FetchFn } from './range-reader.ts'
import { readTextHead } from './text.ts'

export interface FileOps {
  /** 앞에서 maxBytes 만 읽어 디코딩한다. 현재 표시 중인 파일만 읽힌다(프로토콜이 검증) */
  readText(path: string, maxBytes: number): Promise<TextResult>
  /** zip 계열·tar·tar.gz 의 항목 목록(압축 해제 없이) */
  listArchive(path: string, max: number): Promise<ArchiveListing>
}

/** fileUrl 은 호스트별 파일 URL 규칙(peek://file/…, http://peek.localhost/…) */
export function createFileOps(fileUrl: (path: string) => string, fetchFn?: FetchFn): FileOps {
  return {
    readText: (path, maxBytes) => readTextHead(createRangeReader(fileUrl(path), fetchFn), maxBytes),
    listArchive: (path, max) => listArchive(createRangeReader(fileUrl(path), fetchFn), path, max)
  }
}
