// peek 프로토콜을 흉내 내는 fake fetch. 파일은 실제 디스크에서 읽는다.
// 서버가 Range 를 어떻게 다루든 range-reader 가 같은 결과를 내는지 보려고 동작 모드를 둔다.

import { readFile } from 'node:fs/promises'
import { fromPeekUrl, toPeekUrl } from '../../src/shared/channels.ts'
import { parseRange } from '../../src/main/range.ts'

export type Mode =
  /** 표준 동작: 206 + Content-Range, 범위 밖은 416 */
  | 'range'
  /** Range 를 무시하고 200 으로 전체를 준다(Content-Length 있음) */
  | 'ignore'
  /** 위와 같지만 Content-Length 가 없다 */
  | 'ignore-nolen'
  /** 206 이지만 응답마다 최대 700바이트만 준다(응답 크기 상한이 있는 호스트) */
  | 'chunked'

export const MODES: Mode[] = ['range', 'ignore', 'ignore-nolen', 'chunked']

export interface Fake {
  fetch: (url: string, init?: RequestInit) => Promise<Response>
  fileUrl: (path: string) => string
  requests: string[]
  /** 서버가 실제로 내보낸(소비자가 끌어간) 본문 바이트 총량 */
  readonly bytesSent: number
}

export function createFakeFetch(mode: Mode, files?: Map<string, Buffer>): Fake {
  const requests: string[] = []
  let bytesSent = 0
  const load = async (p: string): Promise<Buffer> => files?.get(p) ?? readFile(p)
  return {
    requests,
    get bytesSent() {
      return bytesSent
    },
    fileUrl: toPeekUrl,
    async fetch(url, init) {
      const path = fromPeekUrl(url)
      if (path === null) return new Response('Forbidden', { status: 403 })
      const data = await load(path)
      const range = (init?.headers as Record<string, string> | undefined)?.Range ?? null
      requests.push(range ?? '(full)')
      const r = mode === 'ignore' || mode === 'ignore-nolen' ? ({ kind: 'none' } as const) : parseRange(range, data.length)
      if (r.kind === 'unsatisfiable') return new Response(null, { status: 416, headers: { 'Content-Range': `bytes */${data.length}` } })
      if (r.kind === 'ok') {
        const end = mode === 'chunked' ? Math.min(r.end, r.start + 699) : r.end
        bytesSent += end - r.start + 1
        return new Response(new Uint8Array(data.subarray(r.start, end + 1)), {
          status: 206,
          headers: { 'Content-Range': `bytes ${r.start}-${end}/${data.length}`, 'Content-Length': String(end - r.start + 1) }
        })
      }
      // 전체 본문은 64KB 조각으로 소비자가 끌어갈 때만 내보낸다(중간에 cancel 하면 나머지는 전송되지 않는다)
      let sent = 0
      const body = new ReadableStream<Uint8Array>(
        {
          pull(ctrl) {
            if (sent >= data.length) return ctrl.close()
            const piece = new Uint8Array(data.subarray(sent, sent + 65536))
            sent += piece.length
            bytesSent += piece.length
            ctrl.enqueue(piece)
          }
        },
        { highWaterMark: 0 }
      )
      return new Response(body, { status: 200, headers: mode === 'ignore-nolen' ? {} : { 'Content-Length': String(data.length) } })
    }
  }
}
