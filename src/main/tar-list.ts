import { createReadStream } from 'node:fs'
import type { FileHandle } from 'node:fs/promises'
import { pipeline } from 'node:stream'
import { createGunzip } from 'node:zlib'
import type { ArchiveEntry, ArchiveListing } from '../shared/preview-api'

// ---------- tar / tar.gz 목록 ----------
// tar 는 색인이 없어 512바이트 헤더를 처음부터 차례로 읽는다. 파일 본문은 건너뛴다.
// 평문 tar 는 파일 위치를 옮기며 헤더만 읽으므로 크기와 무관하게 빠르다.
// gzip 은 본문도 풀어야 다음 헤더에 닿으므로 해제량에 상한을 둔다.

const BLOCK = 512
/** tar.gz 를 풀어 볼 최대 바이트. 넘으면 그때까지 읽은 항목만 보여 준다 */
const GZ_MAX_SCAN_BYTES = 256 * 1024 * 1024
/** GNU 긴 이름(L)·pax(x) 같은 메타 블록 최대 크기 */
const META_MAX = 1024 * 1024
const MAX_NAME = 1024
export const TAR_CORRUPT = '손상되었거나 tar 형식이 아닌 파일입니다'
const GZ_NOT_TAR = 'gzip 으로 압축한 단일 파일이라 목록이 없습니다'

/** 헤더에 기록된 체크섬이 맞으면 tar 헤더로 본다(평문 tar 판별용). */
export function isTarHeader(h: Buffer): boolean {
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

export function isGzip(head: Buffer): boolean {
  return head.length >= 2 && head[0] === 0x1f && head[1] === 0x8b
}

interface ByteSource {
  /** n 바이트를 읽는다. 끝에 닿으면 더 짧게 돌려준다 */
  read(n: number): Promise<Buffer>
  /** n 바이트를 버린다. 끝 또는 상한에 닿으면 false */
  skip(n: number): Promise<boolean>
  /** 해제 상한에 닿았는지 */
  readonly limited: boolean
  close(): void
}

function fileSource(fh: FileHandle, size: number): ByteSource {
  let pos = 0
  return {
    async read(n) {
      const len = Math.max(0, Math.min(n, size - pos))
      const buf = Buffer.alloc(len)
      const { bytesRead } = len > 0 ? await fh.read(buf, 0, len, pos) : { bytesRead: 0 }
      pos += bytesRead
      return buf.subarray(0, bytesRead)
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

function gzipSource(path: string, cap: number): ByteSource {
  const gunzip = createGunzip()
  // pipeline 이 원본 읽기 오류도 gunzip 쪽으로 전달한다(오류는 아래 it.next() 에서 드러남)
  pipeline(createReadStream(path), gunzip, () => {})
  const it = gunzip[Symbol.asyncIterator]() as AsyncIterator<Buffer>
  let buf: Buffer = Buffer.alloc(0)
  let ended = false
  let consumed = 0
  let limited = false

  const pull = async (): Promise<boolean> => {
    if (ended) return false
    const r = await it.next()
    if (r.done) {
      ended = true
      return false
    }
    buf = buf.length > 0 ? Buffer.concat([buf, r.value]) : r.value
    return true
  }

  return {
    async read(n) {
      if (consumed > cap) {
        limited = true
        return Buffer.alloc(0)
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
      gunzip.destroy()
    }
  }
}

/** 8진수 텍스트 또는 GNU base-256(첫 바이트 최상위 비트) 숫자. 형식이 틀리면 null */
function parseNumber(b: Buffer): number | null {
  if (b.length > 0 && (b[0] & 0x80) !== 0) {
    if ((b[0] & 0x40) !== 0) return null // 음수
    let v = b[0] & 0x3f
    for (let i = 1; i < b.length; i++) {
      v = v * 256 + b[i]
      if (v > Number.MAX_SAFE_INTEGER) return null
    }
    return v
  }
  const s = b.toString('latin1').replace(/\0[\s\S]*$/, '').trim()
  if (s === '') return 0
  if (!/^[0-7]+$/.test(s)) return null
  const v = parseInt(s, 8)
  return v <= Number.MAX_SAFE_INTEGER ? v : null
}

function cString(b: Buffer): Buffer {
  const nul = b.indexOf(0)
  return nul >= 0 ? b.subarray(0, nul) : b
}

/** pax 확장 헤더("길이 키=값\n" 반복)에서 필요한 키만 뽑는다. 값은 UTF-8 */
function parsePax(data: Buffer): Map<string, string> {
  const out = new Map<string, string>()
  let pos = 0
  while (pos < data.length) {
    const sp = data.indexOf(0x20, pos)
    if (sp < 0) break
    const len = parseInt(data.toString('latin1', pos, sp), 10)
    if (!Number.isFinite(len) || len <= 0 || pos + len > data.length) break
    const rec = data.subarray(sp + 1, pos + len - 1) // 끝의 "\n" 제외
    const eq = rec.indexOf(0x3d)
    if (eq > 0) out.set(rec.toString('utf8', 0, eq), rec.toString('utf8', eq + 1))
    pos += len
  }
  return out
}

/** 본문이 뒤따르는 항목 종류(일반 파일·연속 파일·GNU sparse). 링크·디렉터리·장치는 본문이 없다 */
const DATA_TYPES = new Set(['0', '\0', '7', 'S'])
/** 알려진 종류. 모르는 종류는 본문이 있다고 보고 건너뛴다(Python tarfile 과 같은 규칙) */
const KNOWN_TYPES = new Set(['0', '\0', '1', '2', '3', '4', '5', '6', '7', 'S'])

export async function listTar(
  fh: FileHandle,
  path: string,
  size: number,
  gzip: boolean,
  limit: number,
  decodeName: (bytes: Buffer) => string
): Promise<ArchiveListing> {
  const src = gzip ? gzipSource(path, GZ_MAX_SCAN_BYTES) : fileSource(fh, size)
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
      const paxSize = pax.has('size') ? Number(pax.get('size')) : NaN
      const entrySize = Number.isSafeInteger(paxSize) && paxSize >= 0 ? paxSize : headerSize
      const padded = Math.ceil(entrySize / BLOCK) * BLOCK

      // 메타 블록: 다음 항목의 이름·크기 등을 담는다
      if (type === 'L' || type === 'K' || type === 'x' || type === 'g') {
        if (entrySize > META_MAX) throw new Error(TAR_CORRUPT)
        const data = await src.read(padded)
        if (data.length < padded) throw new Error(TAR_CORRUPT)
        if (type === 'L') longName = decodeName(cString(data.subarray(0, entrySize)))
        else if (type === 'x') pax = parsePax(data.subarray(0, entrySize))
        continue // K(긴 링크 대상)·g(전역 pax)는 목록에 쓰지 않는다
      }

      let name = pax.get('path') ?? longName
      if (name === undefined || name === null) {
        name = decodeName(cString(h.subarray(0, 100)))
        // ustar: 이름이 100바이트를 넘으면 앞부분이 prefix(345~500)에 들어간다
        if (h.toString('latin1', 257, 262) === 'ustar') {
          const prefix = cString(h.subarray(345, 500))
          if (prefix.length > 0) name = `${decodeName(prefix)}/${name}`
        }
      }
      const paxMtime = pax.has('mtime') ? Number(pax.get('mtime')) : NaN
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
