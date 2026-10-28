import { open, readdir, stat } from 'node:fs/promises'
import { basename, extname, join } from 'node:path'
import { crc32 } from 'node:zlib'
import type { ArchiveEntry, ArchiveListing, DirEntry, FileInfo, TextResult } from '../shared/preview-api'

export async function getFileInfo(path: string): Promise<FileInfo> {
  const st = await stat(path)
  const name = basename(path) || path
  return {
    path,
    name,
    ext: st.isDirectory() ? '' : fileExt(name),
    size: st.size,
    isDirectory: st.isDirectory(),
    mtimeMs: st.mtimeMs
  }
}

/** "a.PNG" → "png", ".gitignore" → "gitignore", "Makefile" → "" */
function fileExt(name: string): string {
  const ext = extname(name)
  if (ext) return ext.slice(1).toLowerCase()
  return name.startsWith('.') ? name.slice(1).toLowerCase() : ''
}

/** 앞에서 maxBytes 만 읽어 텍스트로 디코딩. BOM → UTF-8 검증 → 실패 시 EUC-KR(CP949) 순으로 판단. */
export async function readTextHead(path: string, maxBytes: number): Promise<TextResult> {
  const limit = Math.max(0, Math.min(maxBytes, 16 * 1024 * 1024))
  const fh = await open(path, 'r')
  try {
    const size = (await fh.stat()).size
    const buf = Buffer.alloc(Math.min(size, limit))
    const { bytesRead } = await fh.read(buf, 0, buf.length, 0)
    const bytes = buf.subarray(0, bytesRead)
    const truncated = size > bytesRead
    const { text, encoding } = decode(bytes, truncated)
    return { text, truncated, encoding }
  } finally {
    await fh.close()
  }
}

function decode(bytes: Buffer, truncated: boolean): { text: string; encoding: string } {
  if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf)
    return { text: new TextDecoder('utf-8').decode(bytes.subarray(3)), encoding: 'utf-8' }
  if (bytes[0] === 0xff && bytes[1] === 0xfe)
    return { text: new TextDecoder('utf-16le').decode(bytes.subarray(2)), encoding: 'utf-16le' }
  if (bytes[0] === 0xfe && bytes[1] === 0xff)
    return { text: new TextDecoder('utf-16be').decode(bytes.subarray(2)), encoding: 'utf-16be' }

  // 잘린 경우 마지막 멀티바이트 문자가 끊겼을 수 있으니 끝 몇 바이트는 검증에서 뺀다
  const check = truncated ? bytes.subarray(0, Math.max(0, bytes.length - 4)) : bytes
  try {
    new TextDecoder('utf-8', { fatal: true }).decode(check)
    return { text: new TextDecoder('utf-8').decode(bytes), encoding: 'utf-8' }
  } catch {
    return { text: new TextDecoder('euc-kr').decode(bytes), encoding: 'euc-kr' }
  }
}

export async function listDirectory(path: string, max: number): Promise<{ entries: DirEntry[]; truncated: boolean }> {
  const dirents = await readdir(path, { withFileTypes: true })
  const slice = dirents.slice(0, Math.max(0, max))
  const entries = await Promise.all(
    slice.map(async (d): Promise<DirEntry> => {
      try {
        const st = await stat(join(path, d.name))
        return { name: d.name, isDirectory: st.isDirectory(), size: st.size, mtimeMs: st.mtimeMs }
      } catch {
        return { name: d.name, isDirectory: d.isDirectory(), size: 0, mtimeMs: 0 }
      }
    })
  )
  return { entries, truncated: dirents.length > slice.length }
}

// ---------- zip 계열 압축 파일 목록 ----------

const ZIP_MAX_CD_BYTES = 64 * 1024 * 1024
/** 항목 이름 최대 길이(문자). 정상 경로는 Windows MAX_PATH(260)·NTFS 한계(32767)보다 훨씬 짧다 */
const ZIP_MAX_NAME = 1024
/** EOCD(22) + 최대 주석(65535) */
const ZIP_EOCD_SEARCH = 65557
const ZIP_CORRUPT = '손상되었거나 zip 형식이 아닌 파일입니다'

/**
 * 압축을 풀지 않고 중앙 디렉터리만 읽어 항목 목록을 만든다(ZIP64 지원).
 * 파일 전체를 읽지 않고 끝부분(EOCD) → 중앙 디렉터리 범위만 읽는다. 항목은 max 개까지만 파싱한다.
 */
export async function listArchive(path: string, max: number): Promise<ArchiveListing> {
  const limit = Math.max(0, Math.min(Math.floor(max), 100_000))
  const fh = await open(path, 'r')
  try {
    const size = (await fh.stat()).size
    if (size < 22) throw new Error(ZIP_CORRUPT)

    const readAt = async (pos: number, len: number): Promise<Buffer> => {
      const buf = Buffer.alloc(len)
      const { bytesRead } = await fh.read(buf, 0, len, pos)
      return bytesRead === len ? buf : buf.subarray(0, bytesRead)
    }

    // 1) 파일 끝에서 역방향으로 EOCD 서명 탐색
    const tailLen = Math.min(size, ZIP_EOCD_SEARCH)
    const tailPos = size - tailLen
    const tail = await readAt(tailPos, tailLen)
    let eocd = -1
    for (let i = tail.length - 22; i >= 0; i--) {
      if (tail.readUInt32LE(i) !== 0x06054b50) continue
      // 주석 길이가 파일 끝과 정확히 맞는 후보를 우선(주석 안에 서명이 우연히 들어간 경우 배제)
      if (i + 22 + tail.readUInt16LE(i + 20) === tail.length) {
        eocd = i
        break
      }
      if (eocd < 0) eocd = i
    }
    if (eocd < 0) throw new Error(ZIP_CORRUPT)
    const eocdAbs = tailPos + eocd

    let totalEntries = tail.readUInt16LE(eocd + 10)
    let cdSize = tail.readUInt32LE(eocd + 12)
    let cdOffset = tail.readUInt32LE(eocd + 16)
    const commentLen = Math.min(tail.readUInt16LE(eocd + 20), tail.length - eocd - 22)
    const commentBytes = tail.subarray(eocd + 22, eocd + 22 + commentLen)

    // 2) ZIP64: EOCD 바로 앞의 locator → EOCD64 레코드의 64비트 값을 우선한다
    if (eocdAbs >= 20) {
      const loc = await readAt(eocdAbs - 20, 20)
      if (loc.length === 20 && loc.readUInt32LE(0) === 0x07064b50) {
        const rec64Pos = safeNumber(loc.readBigUInt64LE(8))
        const rec = await readAt(rec64Pos, 56)
        if (rec.length < 56 || rec.readUInt32LE(0) !== 0x06064b50) throw new Error(ZIP_CORRUPT)
        totalEntries = safeNumber(rec.readBigUInt64LE(32))
        cdSize = safeNumber(rec.readBigUInt64LE(40))
        cdOffset = safeNumber(rec.readBigUInt64LE(48))
      }
    }

    if (cdSize > ZIP_MAX_CD_BYTES) throw new Error('압축 파일의 목록 정보가 너무 커서 표시할 수 없습니다')
    if (cdOffset + cdSize > size) throw new Error(ZIP_CORRUPT)

    // 3) 중앙 디렉터리 읽기. 앞에 다른 데이터가 붙은 파일(자체 압축 풀기 exe 등)은 오프셋이 어긋나므로 EOCD 기준으로 보정
    let cd = await readAt(cdOffset, cdSize)
    if (cdSize > 0 && (cd.length < 4 || cd.readUInt32LE(0) !== 0x02014b50)) {
      const shifted = eocdAbs - cdSize
      if (shifted < 0) throw new Error(ZIP_CORRUPT)
      cd = await readAt(shifted, cdSize)
    }
    // 중앙 디렉터리 항목 하나의 최소 크기는 46바이트
    if (cd.length !== cdSize || (cdSize > 0 && (cdSize < 46 || cd.readUInt32LE(0) !== 0x02014b50))) throw new Error(ZIP_CORRUPT)

    const want = Math.min(totalEntries, limit)
    const entries: ArchiveEntry[] = []
    let totalSize = 0
    let compressedSize = 0
    let encrypted = false
    let pos = 0
    while (entries.length < want) {
      if (pos + 46 > cd.length || cd.readUInt32LE(pos) !== 0x02014b50) throw new Error(ZIP_CORRUPT)
      const flags = cd.readUInt16LE(pos + 8)
      const method = cd.readUInt16LE(pos + 10)
      const dosTime = cd.readUInt16LE(pos + 12)
      const dosDate = cd.readUInt16LE(pos + 14)
      let compSize = cd.readUInt32LE(pos + 20)
      let rawSize = cd.readUInt32LE(pos + 24)
      const nameLen = cd.readUInt16LE(pos + 28)
      const extraLen = cd.readUInt16LE(pos + 30)
      const cmtLen = cd.readUInt16LE(pos + 32)
      const next = pos + 46 + nameLen + extraLen + cmtLen
      if (next > cd.length) throw new Error(ZIP_CORRUPT)
      const nameBytes = cd.subarray(pos + 46, pos + 46 + nameLen)
      const extra = cd.subarray(pos + 46 + nameLen, pos + 46 + nameLen + extraLen)

      let name: string | null = null
      let mtimeMs = 0
      for (let e = 0; e + 4 <= extra.length; ) {
        const id = extra.readUInt16LE(e)
        const len = extra.readUInt16LE(e + 2)
        const data = extra.subarray(e + 4, Math.min(extra.length, e + 4 + len))
        if (id === 0x0001) {
          // ZIP64: 해당 필드가 0xFFFFFFFF 로 포화된 것만 순서대로(원본 → 압축) 들어 있다
          let o = 0
          if (rawSize === 0xffffffff && o + 8 <= data.length) {
            rawSize = safeNumber(data.readBigUInt64LE(o))
            o += 8
          }
          if (compSize === 0xffffffff && o + 8 <= data.length) compSize = safeNumber(data.readBigUInt64LE(o))
        } else if (id === 0x5455 && data.length >= 5 && (data[0] & 1) !== 0) {
          // Extended Timestamp: UTC 초 단위 수정 시각. DOS 시각(로컬·2초 단위)보다 정확하다
          mtimeMs = data.readUInt32LE(1) * 1000
        } else if (id === 0x7075 && data.length > 5 && data[0] === 1 && data.readUInt32LE(1) === crc32(nameBytes)) {
          // Info-ZIP Unicode Path: 원래 이름 바이트의 CRC 가 맞을 때만 쓴다(명세). 맞지 않는 값을 믿으면
          // 실제로는 "x.pdf.exe" 로 풀리는 항목을 미리보기에서 "x.pdf" 로 보이게 위장할 수 있다.
          name = new TextDecoder('utf-8').decode(data.subarray(5))
        }
        e += 4 + len
      }
      if (name === null) name = decodeZipName(nameBytes, (flags & 0x0800) !== 0)
      name = name.replace(/\\/g, '/') // Compress-Archive(PS 5.1) 등은 역슬래시로 저장한다
      if (name.length > ZIP_MAX_NAME) name = name.slice(0, ZIP_MAX_NAME) + '…' // 아주 깊은 경로로 렌더러 트리를 부풀리는 입력 방지
      if (mtimeMs === 0) mtimeMs = dosToMs(dosDate, dosTime)

      const isDirectory = name.endsWith('/')
      const isEncrypted = (flags & 1) !== 0
      if (isEncrypted) encrypted = true
      if (!isDirectory) {
        totalSize += rawSize
        compressedSize += compSize
      }
      entries.push({ path: name, isDirectory, size: rawSize, compressedSize: compSize, mtimeMs, encrypted: isEncrypted, method })
      pos = next
    }

    const comment = commentBytes.length > 0 ? decodeZipName(commentBytes, false).slice(0, 4096).trim() : ''
    return {
      entries,
      totalEntries,
      truncated: totalEntries > entries.length,
      totalSize,
      compressedSize,
      encrypted,
      ...(comment ? { comment } : {})
    }
  } finally {
    await fh.close()
  }
}

function safeNumber(v: bigint): number {
  if (v > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error(ZIP_CORRUPT)
  return Number(v)
}

/** UTF-8 플래그 → UTF-8. 아니면 유효한 UTF-8 이면 UTF-8, 아니면 CP949(EUC-KR), 그것도 안 되면 latin1. */
function decodeZipName(bytes: Buffer, utf8Flag: boolean): string {
  if (utf8Flag) return new TextDecoder('utf-8').decode(bytes)
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    try {
      return new TextDecoder('euc-kr', { fatal: true }).decode(bytes)
    } catch {
      return bytes.toString('latin1')
    }
  }
}

/** MS-DOS 날짜/시간(로컬, 2초 단위) → epoch ms. 0 이거나 범위 밖이면 0 */
function dosToMs(date: number, time: number): number {
  if (date === 0) return 0
  const ms = new Date(
    1980 + (date >> 9),
    ((date >> 5) & 15) - 1,
    date & 31,
    time >> 11,
    (time >> 5) & 63,
    (time & 31) * 2
  ).getTime()
  return Number.isNaN(ms) ? 0 : ms
}
