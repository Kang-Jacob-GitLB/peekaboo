// peek:// 응답 조립 중 electron 에 의존하지 않는 부분(헤더·Range 본문). node --test 로 검사한다.

import type { FileHandle } from 'node:fs/promises'
import { Readable } from 'node:stream'

/**
 * 허용된 단일 파일만 서빙하므로 모든 출처에 열어 둬도 노출 범위는 같다.
 * Content-Range 등은 노출 헤더로 적어야 렌더러(다른 출처)의 fetch 가 읽을 수 있다.
 */
export function withPeekHeaders(src: Headers): Headers {
  const headers = new Headers(src)
  headers.set('Access-Control-Allow-Origin', '*')
  headers.set('Access-Control-Expose-Headers', 'Content-Range, Content-Length, Accept-Ranges')
  headers.set('Accept-Ranges', 'bytes')
  return headers
}

// 416: 범위가 파일 밖이다. Content-Range 에 "bytes */크기" 로 전체 크기를 알려 준다
export function unsatisfiableHeaders(size: number): Headers {
  const headers = withPeekHeaders(new Headers())
  headers.set('Content-Range', `bytes */${size}`)
  return headers
}

/** 206: 닫힌 구간 [start, end] */
export function partialHeaders(start: number, end: number, size: number, contentType: string | null): Headers {
  const headers = withPeekHeaders(new Headers())
  if (contentType) headers.set('Content-Type', contentType)
  headers.set('Content-Range', `bytes ${start}-${end}/${size}`)
  headers.set('Content-Length', String(end - start + 1))
  return headers
}

/**
 * 열어 둔 파일 핸들에서 [start, end] 만 읽는 웹 스트림. 핸들은 스트림이 끝나거나 오류가 나거나
 * 소비자가 취소하면 닫힌다(autoClose).
 */
export function rangeBody(handle: FileHandle, start: number, end: number): ReadableStream<Uint8Array> {
  return Readable.toWeb(handle.createReadStream({ start, end, autoClose: true, emitClose: true })) as ReadableStream<Uint8Array>
}
