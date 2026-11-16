import { createRangeReader, type FetchFn } from '../host/file-ops/range-reader.ts'

/** 오피스/PDF 플러그인이 메모리에 통째로 올릴 수 있는 최대 파일 크기 */
export const MAX_BYTES = 100 * 1024 * 1024
/**
 * 한 번에 요청하는 크기. 호스트(특히 Tauri 의 peek 프로토콜)는 응답 본문을 통째로 메모리에 올려 넘기므로
 * 전체를 한 번에 받으면 파일 크기의 몇 배가 순간적으로 상주한다(99MiB PDF 에서 호스트 약 1GB). 조각으로 받아 미리 할당한 버퍼에 채운다.
 */
export const CHUNK_BYTES = 4 * 1024 * 1024

const READ_ERROR = '파일을 읽을 수 없습니다.'
const TOO_LARGE = '파일이 너무 커서(100MB 초과) 미리볼 수 없습니다.'

/** Range 를 무시하고 200 으로 전체를 보낸 응답의 본문을 끝까지 받는다. 상한을 넘으면 더 받지 않고 throw 한다. */
async function readFullBody(res: Response, maxBytes: number): Promise<ArrayBuffer> {
  const declared = res.headers.get('content-length')
  if (declared !== null && Number(declared) > maxBytes) {
    await res.body?.cancel()
    throw new Error(TOO_LARGE)
  }
  const chunks: Uint8Array[] = []
  let total = 0
  const rd = res.body?.getReader()
  while (rd) {
    const r = await rd.read()
    if (r.done) break
    total += r.value.length
    if (total > maxBytes) {
      await rd.cancel()
      throw new Error(TOO_LARGE)
    }
    chunks.push(r.value)
  }
  const buffer = new ArrayBuffer(total)
  const out = new Uint8Array(buffer)
  let o = 0
  for (const c of chunks) {
    out.set(c, o)
    o += c.length
  }
  return buffer
}

/**
 * `url` 의 파일 전체를 4MiB 단위 Range 요청으로 읽어 한 버퍼에 담는다.
 * 크기는 첫 응답의 Content-Range 로 알아내고(별도의 크기 요청이 없다), 상한을 넘으면 더 읽지 않고 throw 한다.
 * 파일이 읽는 도중 줄어 끝이 모자라면 throw 한다.
 * 첫 응답이 200 이면(Range 를 무시하는 서버) 그 본문이 이미 파일 전체이므로 조각 요청을 반복하지 않고 그대로 끝까지 받아 돌려준다.
 */
export async function readWhole(url: string, fetchFn: FetchFn, maxBytes = MAX_BYTES, chunk = CHUNK_BYTES): Promise<ArrayBuffer> {
  const firstRes = await fetchFn(url, { headers: { Range: `bytes=0-${chunk - 1}` } })
  if (firstRes.status === 200) return readFullBody(firstRes, maxBytes)

  // 첫 응답은 range-reader 가 해석하도록 그대로 넘겨 주고, 이후 요청은 원래 fetch 로 보낸다
  let replay: Response | null = firstRes
  const reader = createRangeReader(url, (u, init) => {
    if (replay) {
      const res = replay
      replay = null
      return Promise.resolve(res)
    }
    return fetchFn(u, init)
  })
  const first = await reader.read(0, chunk)
  const total = await reader.size() // 첫 응답이 알려 준 값이 캐시돼 있어 추가 요청이 없다
  if (total > maxBytes) throw new Error(TOO_LARGE)
  if (total === 0) return new ArrayBuffer(0)
  if (first.length === 0) throw new Error(READ_ERROR)

  const buffer = new ArrayBuffer(total)
  const out = new Uint8Array(buffer)
  out.set(first.subarray(0, total))
  let pos = Math.min(first.length, total)
  while (pos < total) {
    const part = await reader.read(pos, Math.min(chunk, total - pos))
    if (part.length === 0) throw new Error(READ_ERROR) // 파일이 줄었다
    out.set(part, pos)
    pos += part.length
  }
  return buffer
}

/** readBinary 가 쓰는 RenderContext 의 일부(플러그인 타입 전체를 끌어오지 않아 node --test 가 이 모듈을 바로 불러올 수 있다) */
interface BinarySource {
  file: { path: string; size: number }
  host: { fileUrl(path: string): string }
  signal: AbortSignal
}

/** peek 프로토콜로 파일 전체를 읽는다. 상한 초과면 읽기 전에 throw 한다. */
export async function readBinary({ file, host, signal }: BinarySource): Promise<ArrayBuffer> {
  if (file.size > MAX_BYTES) throw new Error(TOO_LARGE)
  return readWhole(host.fileUrl(file.path), (u, init) => fetch(u, { ...init, signal }))
}
