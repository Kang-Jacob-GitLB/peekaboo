// tar / tar.gz 목록. main 의 tar-list.ts 를 렌더러로 옮긴 것이다.
// Buffer → Uint8Array, node:zlib 의 createGunzip → DecompressionStream('gzip'), 파일 핸들 → Range 읽기(선읽기 캐시).
// 판정과 상한은 원본 그대로다.
//
// tar 는 색인이 없어 512바이트 헤더를 처음부터 차례로 읽는다. 파일 본문은 건너뛴다.
// 평문 tar 는 파일 위치를 옮기며 헤더만 읽으므로 크기와 무관하게 빠르다.
// gzip 은 본문도 풀어야 다음 헤더에 닿으므로 해제량에 상한을 둔다.

import type { ArchiveEntry, ArchiveListing } from '../../../shared/preview-api'
import { concat, indexOfByte, latin1, utf8 } from './bytes.ts'
import type { RangeReader } from './range-reader.ts'

const BLOCK = 512
/** tar.gz 를 풀어 볼 최대 바이트. 넘으면 그때까지 읽은 항목만 보여 준다 */
export const GZ_MAX_SCAN_BYTES = 256 * 1024 * 1024
/** GNU 긴 이름(L)·pax(x) 같은 메타 블록 최대 크기 */
const META_MAX = 1024 * 1024
const MAX_NAME = 1024
/** 평문 tar 선읽기 크기. 시작은 작게(본문을 건너뛰는 tar 에서 낭비하지 않도록), 순차로 이어질 때만 상한까지 키운다 */
const READ_AHEAD_MIN = 16 * 1024
const READ_AHEAD_MAX = 256 * 1024
/** gzip 원본을 Range 로 받는 단위 */
const GZ_FETCH = 256 * 1024
/** DecompressionStream 에 넣는 조각 크기(압축 폭탄이 한꺼번에 메모리로 풀리지 않게 pull 마다 하나씩) */
const GZ_FEED = 16 * 1024
export const TAR_CORRUPT = '손상되었거나 tar 형식이 아닌 파일입니다'
const GZ_NOT_TAR = 'gzip 으로 압축한 단일 파일이라 목록이 없습니다'

/** 헤더에 기록된 체크섬이 맞으면 tar 헤더로 본다(평문 tar 판별용). */
export function isTarHeader(h: Uint8Array): boolean {
  if (h.length < BLOCK) return false
  const recorded = parseNumber(h.subarray(148, 156))
  if (recorded === null) return false
  let unsigned = 0
  let signed = 0
  for (let i = 0; i < BLOCK; i++) {
    const b = i >= 148 && i < 156 ? 0x20 : h[i]
    unsigned += b
    signed += b > 127 ? b - 256 : b
  }
  return recorded === unsigned || recorded === signed
}

export function isGzip(head: Uint8Array): boolean {
  return head.length >= 2 && head[0] === 0x1f && head[1] === 0x8b
}

interface ByteSource {
  /** n 바이트를 읽는다. 끝에 닿으면 더 짧게 돌려준다 */
  read(n: number): Promise<Uint8Array>
  /** n 바이트를 버린다. 끝 또는 상한에 닿으면 false */
  skip(n: number): Promise<boolean>
  /** 해제 상한에 닿았는지 */
  readonly limited: boolean
  close(): void
}

/** 평문 tar: 요청 위치부터 요청량 + 선읽기만 가져오는 Range 소스. 순차로 읽을 때만 선읽기를 키운다 */
function rangeSource(reader: RangeReader, size: number): ByteSource {
  let pos = 0
  let cacheStart = 0
  let cache: Uint8Array = new Uint8Array(0)
  let ahead = READ_AHEAD_MIN
  return {
    async read(n) {
      const len = Math.max(0, Math.min(n, size - pos))
      const out = new Uint8Array(len)
      let filled = 0
      while (filled < len) {
        const abs = pos + filled
        if (abs < cacheStart || abs >= cacheStart + cache.length) {
          // 직전에 가져온 구간 바로 뒤에서 이어 읽으면 순차 읽기로 보고 선읽기를 키운다. skip 으로 건너뛴 뒤라면 처음 크기로 되돌린다
          ahead = cache.length > 0 && abs === cacheStart + cache.length ? Math.min(ahead * 2, READ_AHEAD_MAX) : READ_AHEAD_MIN
          cacheStart = abs
          cache = await reader.read(abs, Math.min(len - filled + ahead, size - abs))
          if (cache.length === 0) break // 파일이 예상보다 짧음
        }
        const off = abs - cacheStart
        const k = Math.min(len - filled, cache.length - off)
        out.set(cache.subarray(off, off + k), filled)
        filled += k
      }
      pos += filled
      return out.subarray(0, filled)
    },
    async skip(n) {
      if (pos + n > size) return false
      pos += n
      return true
    },
    limited: false,
    close() {}
  }
}

function gzipSource(reader: RangeReader, size: number, cap: number): ByteSource {
  let off = 0
  let fetched: Uint8Array = new Uint8Array(0)
  let fetchedOff = 0
  // 원본은 GZ_FETCH 단위로 받되, DecompressionStream 에는 소비 쪽이 출력을 요구할 때(pull)마다 GZ_FEED 조각 하나만 넣는다
  const raw = new ReadableStream<BufferSource>(
    {
      async pull(ctrl) {
        if (fetchedOff >= fetched.length) {
          fetched = off < size ? await reader.read(off, GZ_FETCH) : new Uint8Array(0)
          fetchedOff = 0
          if (fetched.length === 0) {
            ctrl.close()
            return
          }
          off += fetched.length
        }
        const piece = fetched.subarray(fetchedOff, fetchedOff + GZ_FEED)
        fetchedOff += piece.length
        ctrl.enqueue(piece as Uint8Array<ArrayBuffer>)
      }
    },
    { highWaterMark: 0 }
  )
  // 원본 읽기 오류도 해제 스트림으로 전달된다(오류는 아래 rd.read() 에서 드러남)
  const rd = raw.pipeThrough(new DecompressionStream('gzip')).getReader()
  let buf: Uint8Array = new Uint8Array(0)
  let ended = false
  let consumed = 0
  let limited = false

  const pull = async (): Promise<boolean> => {
    if (ended) return false
    const r = await rd.read()
    if (r.done) {
      ended = true
      return false
    }
    buf = buf.length > 0 ? concat(buf, r.value) : r.value
    return true
  }

  return {
    async read(n) {
      if (consumed > cap) {
        limited = true
        return new Uint8Array(0)
      }
      while (buf.length < n && (await pull()));
      const out = buf.subarray(0, Math.min(n, buf.length))
      buf = buf.subarray(out.length)
      consumed += out.length
      return out
    },
    async skip(n) {
      while (n > 0) {
        if (consumed > cap) {
          limited = true
          return false
        }
        if (buf.length === 0 && !(await pull())) return false
        const k = Math.min(n, buf.length)
        buf = buf.subarray(k)
        n -= k
        consumed += k
      }
      return true
    },
    get limited() {
      return limited
    },
    close() {
      rd.cancel().catch(() => {})
    }
  }
}

/** 8진수 텍스트 또는 GNU base-256(첫 바이트 최상위 비트) 숫자. 형식이 틀리면 null */
function parseNumber(b: Uint8Array): number | null {
  if (b.length > 0 && (b[0] & 0x80) !== 0) {
    if ((b[0] & 0x40) !== 0) return null // 음수
    let v = b[0] & 0x3f
    for (let i = 1; i < b.length; i++) {
      v = v * 256 + b[i]
      if (v > Number.MAX_SAFE_INTEGER) return null
    }
    return v
  }
  const s = latin1(b).replace(/\0[\s\S]*$/, '').trim()
  if (s === '') return 0
  if (!/^[0-7]+$/.test(s)) return null
  const v = parseInt(s, 8)
  return v <= Number.MAX_SAFE_INTEGER ? v : null
}

function cString(b: Uint8Array): Uint8Array {
  const nul = indexOfByte(b, 0)
  return nul >= 0 ? b.subarray(0, nul) : b
}

/** pax 확장 헤더("길이 키=값\n" 반복)에서 필요한 키만 뽑는다. 값은 UTF-8 */
function parsePax(data: Uint8Array): Map<string, string> {
  const out = new Map<string, string>()
  let pos = 0
  while (pos < data.length) {
    const sp = indexOfByte(data, 0x20, pos)
    // 길이는 숫자만 허용하고, 레코드가 공백 뒤에서 "\n" 으로 끝나야 한다. 느슨하게 받으면(parseInt 의 앞쪽 공백 허용)
    // pos 가 1바이트씩만 전진하며 같은 구간을 반복 파싱해 입력 크기의 제곱 시간이 걸린다(렌더러 정지).
    if (sp < 0 || sp - pos > 20) break
    const lenStr = latin1(data, pos, sp)
    if (!/^[1-9][0-9]*$/.test(lenStr)) break
    const len = Number(lenStr)
    if (len < sp - pos + 2 || pos + len > data.length || data[pos + len - 1] !== 0x0a) break
    const rec = data.subarray(sp + 1, pos + len - 1) // 끝의 "\n" 제외
    const eq = indexOfByte(rec, 0x3d)
    if (eq > 0) out.set(utf8(rec.subarray(0, eq)), utf8(rec.subarray(eq + 1)))
    pos += len
  }
  return out
}

/** 본문이 뒤따르는 항목 종류(일반 파일·연속 파일·GNU sparse). 링크·디렉터리·장치는 본문이 없다 */
const DATA_TYPES = new Set(['0', '\0', '7', 'S'])
/** 알려진 종류. 모르는 종류는 본문이 있다고 보고 건너뛴다(Python tarfile 과 같은 규칙) */
const KNOWN_TYPES = new Set(['0', '\0', '1', '2', '3', '4', '5', '6', '7', 'S'])

export async function listTar(
  reader: RangeReader,
  size: number,
  gzip: boolean,
  limit: number,
  decodeName: (bytes: Uint8Array) => string,
  gzMaxScanBytes = GZ_MAX_SCAN_BYTES
): Promise<ArchiveListing> {
  const src = gzip ? gzipSource(reader, size, gzMaxScanBytes) : rangeSource(reader, size)
  const entries: ArchiveEntry[] = []
  let totalSize = 0
  let truncated = false
  let longName: string | null = null
  let pax = new Map<string, string>()
  // 메타 블록(g 등)만 끝없이 이어지는 조작된 파일에서 헤더를 무한정 읽지 않도록
  const maxHeaders = limit * 3 + 100
  let headers = 0
  try {
    for (;;) {
      if (++headers > maxHeaders) {
        truncated = true
        break
      }
      const h = await src.read(BLOCK)
      if (h.length === 0) break // 끝 표시(빈 블록 2개) 없이 끝난 tar 도 흔하다
      if (h.length < BLOCK) {
        if (entries.length === 0) throw new Error(gzip ? GZ_NOT_TAR : TAR_CORRUPT)
        break
      }
      if (h.every((b) => b === 0)) break // 끝 표시
      if (!isTarHeader(h)) {
        if (entries.length === 0) throw new Error(gzip ? GZ_NOT_TAR : TAR_CORRUPT)
        truncated = true // 중간에 깨진 헤더: 앞부분만 보여 준다
        break
      }
      if (entries.length >= limit) {
        truncated = true // 다음 항목이 더 있음
        break
      }

      const type = String.fromCharCode(h[156])
      const headerSize = parseNumber(h.subarray(124, 136))
      if (headerSize === null) throw new Error(TAR_CORRUPT)

      // 메타 블록: 다음 항목의 이름·크기 등을 담는다. 메타 블록 자신의 크기에는 pax size 를 적용하지 않는다
      if (type === 'L' || type === 'K' || type === 'x' || type === 'g') {
        if (headerSize > META_MAX) throw new Error(TAR_CORRUPT)
        const metaPadded = Math.ceil(headerSize / BLOCK) * BLOCK
        const data = await src.read(metaPadded)
        if (data.length < metaPadded) throw new Error(TAR_CORRUPT)
        if (type === 'L') longName = decodeName(cString(data.subarray(0, headerSize)))
        else if (type === 'x') pax = parsePax(data.subarray(0, headerSize))
        continue // K(긴 링크 대상)·g(전역 pax)는 목록에 쓰지 않는다
      }

      // pax 숫자는 10진 숫자만 받는다. Number() 는 "1e3"·"0x200" 도 받아들여 실제 압축 해제 도구(libarchive)와
      // 다음 헤더 위치가 어긋나므로, 미리보기에만 보이는 가짜 항목을 만들 여지가 생긴다
      const paxSizeStr = pax.get('size')
      const entrySize = paxSizeStr !== undefined && /^[0-9]{1,16}$/.test(paxSizeStr) ? Number(paxSizeStr) : headerSize
      const padded = Math.ceil(entrySize / BLOCK) * BLOCK

      let name = pax.get('path') ?? longName
      if (name === undefined || name === null) {
        name = decodeName(cString(h.subarray(0, 100)))
        // ustar: 이름이 100바이트를 넘으면 앞부분이 prefix(345~500)에 들어간다
        if (latin1(h, 257, 262) === 'ustar') {
          const prefix = cString(h.subarray(345, 500))
          if (prefix.length > 0) name = `${decodeName(prefix)}/${name}`
        }
      }
      const paxMtimeStr = pax.get('mtime')
      const paxMtime = paxMtimeStr !== undefined && /^-?[0-9]{1,15}(\.[0-9]+)?$/.test(paxMtimeStr) ? Number(paxMtimeStr) : NaN
      const mtime = Number.isFinite(paxMtime) ? paxMtime : (parseNumber(h.subarray(136, 148)) ?? 0)
      longName = null
      pax = new Map()

      if (name.length > MAX_NAME) name = name.slice(0, MAX_NAME) + '…'
      const isDirectory = type === '5' || name.endsWith('/')
      const hasData = DATA_TYPES.has(type) || !KNOWN_TYPES.has(type)
      const fileSize = hasData && !isDirectory ? entrySize : 0
      totalSize += fileSize
      entries.push({
        path: name,
        isDirectory,
        size: fileSize,
        compressedSize: fileSize,
        mtimeMs: Math.max(0, Math.floor(mtime * 1000)),
        encrypted: false,
        method: 0
      })

      // 구형 GNU sparse: isextended(482) 가 켜져 있으면 확장 블록이 이어진다. 각 블록의 504 바이트가 다음 확장 여부
      if (type === 'S' && h[482] !== 0) {
        let ext: Uint8Array
        do {
          if (++headers > maxHeaders) throw new Error(TAR_CORRUPT)
          ext = await src.read(BLOCK)
          if (ext.length < BLOCK) throw new Error(TAR_CORRUPT)
        } while (ext[504] !== 0)
      }

      if (hasData && !(await src.skip(padded))) {
        // 본문 중간에 파일이 끝남(잘린 tar) 또는 gzip 해제 상한
        truncated = src.limited || truncated
        break
      }
    }
  } catch (e) {
    // gzip 손상 등으로 중간에 실패하면 읽은 데까지는 보여 준다
    if (entries.length === 0) throw e instanceof Error && (e.message === TAR_CORRUPT || e.message === GZ_NOT_TAR) ? e : new Error(TAR_CORRUPT)
    truncated = true
  } finally {
    src.close()
  }

  return {
    entries,
    // tar 에는 전체 항목 수 기록이 없다. 잘렸으면 읽은 수가 하한이다
    totalEntries: entries.length,
    totalIsLowerBound: truncated,
    truncated,
    scanLimited: src.limited,
    totalSize,
    // 평문 tar 는 압축이 없다. tar.gz 는 끝까지 읽었을 때만 파일 크기가 곧 압축 크기다
    compressedSize: gzip && !truncated ? size : totalSize,
    encrypted: false
  }
}
