// peek 프로토콜에서 파일의 일부 바이트를 읽는다. 파일 해석(텍스트·압축 목록)은 이 모듈로만 바이트를 가져온다.
// 보안 경계는 프로토콜(main)이 지킨다: 현재 미리보기 중인 파일 하나만 서빙하므로 여기서는 경로를 따지지 않는다.

export type FetchFn = (url: string, init?: RequestInit) => Promise<Response>

export interface RangeReader {
  /** 파일 전체 크기 */
  size(): Promise<number>
  /** [pos, pos+len) 바이트. 파일 끝을 넘으면 짧게, 시작이 끝 너머면 빈 배열 */
  read(pos: number, len: number): Promise<Uint8Array>
}

const READ_ERROR = '파일을 읽을 수 없습니다'
/** Range 를 무시하는 서버에서 한 번에 모아 두는 최대 바이트(호출부가 이어 읽는다) */
const FALLBACK_MAX_BYTES = 16 * 1024 * 1024

function concatChunks(chunks: Uint8Array[], total: number): Uint8Array {
  if (chunks.length === 1) return chunks[0]
  const out = new Uint8Array(total)
  let o = 0
  for (const c of chunks) {
    out.set(c, o)
    o += c.length
  }
  return out
}

export function createRangeReader(url: string, fetchFn: FetchFn = (u, init) => fetch(u, init)): RangeReader {
  let total: number | null = null

  /** 요청 한 번. 서버가 일부만 줄 수 있으므로(응답 크기 상한) 호출부가 이어 읽는다 */
  async function fetchOnce(pos: number, len: number): Promise<Uint8Array> {
    const res = await fetchFn(url, { headers: { Range: `bytes=${pos}-${pos + len - 1}` } })

    if (res.status === 206) {
      // Content-Range: bytes a-b/total. 요청한 시작과 다르면 엉뚱한 구간을 파일 내용으로 해석하게 되므로 거부한다
      const m = /^bytes (\d+)-(\d+)\/(\d+|\*)$/.exec(res.headers.get('content-range') ?? '')
      if (!m || Number(m[1]) !== pos) {
        await res.body?.cancel()
        throw new Error(READ_ERROR)
      }
      if (m[3] !== '*') total = Number(m[3])
      const bytes = new Uint8Array(await res.arrayBuffer())
      return bytes.length > len ? bytes.subarray(0, len) : bytes
    }

    if (res.status === 416) {
      // 범위가 파일 밖(빈 파일 포함). Content-Range: bytes */total 로 크기를 알려 준다
      const m = /^bytes \*\/(\d+)$/.exec(res.headers.get('content-range') ?? '')
      await res.body?.cancel()
      if (m) total = Number(m[1])
      return new Uint8Array(0)
    }

    if (res.status === 200) {
      // Range 를 무시하고 전체를 보냈다. 본문은 파일 처음부터이므로 흘려 보내며 pos 앞은 버리고 필요한 구간만 모은 뒤 닫는다
      const declared = Number(res.headers.get('content-length'))
      if (res.headers.get('content-length') !== null && Number.isSafeInteger(declared)) total = declared
      const wantLen = Math.min(len, FALLBACK_MAX_BYTES) // 모으는 양의 상한
      const end = pos + wantLen
      const chunks: Uint8Array[] = []
      let seen = 0 // 지금까지 본 바이트(버린 것 포함)
      let kept = 0
      const rd = res.body?.getReader()
      let finished = false
      if (rd) {
        while (total === null || seen < end) {
          const r = await rd.read()
          if (r.done) {
            finished = true
            break
          }
          const from = Math.max(pos, seen)
          const to = Math.min(end, seen + r.value.length)
          if (to > from) {
            chunks.push(r.value.subarray(from - seen, to - seen))
            kept += to - from
          }
          seen += r.value.length
          // 길이를 아는 경우에만 필요한 만큼 받고 멈춘다. 모르면 크기를 알아내려고 끝까지 읽는다(모으지는 않는다)
          if (total !== null && seen >= end) break
        }
        if (!finished) await rd.cancel()
      }
      if (finished || !rd) total = seen // 길이를 모르면 끝까지 읽은 길이가 곧 크기다
      return kept === 0 ? new Uint8Array(0) : concatChunks(chunks, kept)
    }

    await res.body?.cancel()
    throw new Error(`${READ_ERROR} (HTTP ${res.status})`)
  }

  return {
    async size() {
      if (total === null) await fetchOnce(0, 1)
      if (total === null) throw new Error(READ_ERROR)
      return total
    },
    async read(pos, len) {
      const out: Uint8Array[] = []
      let got = 0
      for (;;) {
        let want = len - got
        if (total !== null) want = Math.min(want, total - pos - got)
        if (want <= 0) break
        const part = await fetchOnce(pos + got, want)
        if (part.length === 0) break // 파일 끝(또는 더 줄 수 없음): 무한 반복 방지
        out.push(part)
        got += part.length
      }
      return got === 0 ? new Uint8Array(0) : concatChunks(out, got)
    }
  }
}
