// zip 계열 압축 파일 목록. main 의 file-service.ts(listArchive 의 zip 부분)를 렌더러로 옮긴 것이다.
// Buffer → Uint8Array/DataView, node:zlib crc32 → bytes.ts 의 표 구현. 판정과 상한은 원본 그대로다.

import type { ArchiveEntry, ArchiveListing } from '../../../shared/preview-api'
import { ZIP_CORRUPT, crc32, decodeZipName, safeNumber, view } from './bytes.ts'
import type { RangeReader } from './range-reader.ts'

/** 중앙 디렉터리를 이만큼만 읽는다. 더 크면 앞부분 항목만 보여 준다 */
export const ZIP_MAX_CD_BYTES = 64 * 1024 * 1024
/** 항목 이름 최대 길이(문자). 정상 경로는 Windows MAX_PATH(260)·NTFS 한계(32767)보다 훨씬 짧다 */
const ZIP_MAX_NAME = 1024
/** EOCD(22) + 최대 주석(65535) */
const ZIP_EOCD_SEARCH = 65557

export async function listZip(reader: RangeReader, size: number, limit: number, maxCdBytes = ZIP_MAX_CD_BYTES): Promise<ArchiveListing> {
  if (size < 22) throw new Error(ZIP_CORRUPT)

  // 1) 파일 끝에서 역방향으로 EOCD 서명 탐색
  const tailLen = Math.min(size, ZIP_EOCD_SEARCH)
  const tailPos = size - tailLen
  const tail = await reader.read(tailPos, tailLen)
  const tv = view(tail)
  let eocd = -1
  for (let i = tail.length - 22; i >= 0; i--) {
    if (tv.getUint32(i, true) !== 0x06054b50) continue
    // 주석 길이가 파일 끝과 정확히 맞는 후보를 우선(주석 안에 서명이 우연히 들어간 경우 배제)
    if (i + 22 + tv.getUint16(i + 20, true) === tail.length) {
      eocd = i
      break
    }
    if (eocd < 0) eocd = i
  }
  if (eocd < 0) throw new Error(ZIP_CORRUPT)
  const eocdAbs = tailPos + eocd

  let totalEntries = tv.getUint16(eocd + 10, true)
  let cdSize = tv.getUint32(eocd + 12, true)
  let cdOffset = tv.getUint32(eocd + 16, true)
  const commentLen = Math.min(tv.getUint16(eocd + 20, true), tail.length - eocd - 22)
  const commentBytes = tail.subarray(eocd + 22, eocd + 22 + commentLen)

  // 2) ZIP64: EOCD 바로 앞의 locator → EOCD64 레코드의 64비트 값을 우선한다
  if (eocdAbs >= 20) {
    const loc = await reader.read(eocdAbs - 20, 20)
    const lv = view(loc)
    if (loc.length === 20 && lv.getUint32(0, true) === 0x07064b50) {
      const rec64Pos = safeNumber(lv.getBigUint64(8, true))
      const rec = await reader.read(rec64Pos, 56)
      const rv = view(rec)
      if (rec.length < 56 || rv.getUint32(0, true) !== 0x06064b50) throw new Error(ZIP_CORRUPT)
      totalEntries = safeNumber(rv.getBigUint64(32, true))
      cdSize = safeNumber(rv.getBigUint64(40, true))
      cdOffset = safeNumber(rv.getBigUint64(48, true))
    }
  }

  if (cdOffset + cdSize > size) throw new Error(ZIP_CORRUPT)

  // 3) 중앙 디렉터리 읽기(앞부분 최대 maxCdBytes). 앞에 다른 데이터가 붙은 파일(자체 압축 풀기 exe 등)은
  //    오프셋이 어긋나므로 EOCD 기준으로 보정
  const readLen = Math.min(cdSize, maxCdBytes)
  const cdPartial = readLen < cdSize
  let cd = await reader.read(cdOffset, readLen)
  if (cdSize > 0 && (cd.length < 4 || view(cd).getUint32(0, true) !== 0x02014b50)) {
    const shifted = eocdAbs - cdSize
    if (shifted < 0) throw new Error(ZIP_CORRUPT)
    cd = await reader.read(shifted, readLen)
  }
  // 중앙 디렉터리 항목 하나의 최소 크기는 46바이트
  if (cd.length !== readLen || (cdSize > 0 && (cdSize < 46 || view(cd).getUint32(0, true) !== 0x02014b50))) throw new Error(ZIP_CORRUPT)
  const cv = view(cd)

  const want = Math.min(totalEntries, limit)
  const entries: ArchiveEntry[] = []
  let totalSize = 0
  let compressedSize = 0
  let encrypted = false
  let pos = 0
  let scanLimited = false
  while (entries.length < want) {
    // 앞부분만 읽은 경우 버퍼 끝에 걸린 항목에서 멈춘다
    if (cdPartial && pos + 46 > cd.length) {
      scanLimited = true
      break
    }
    if (pos + 46 > cd.length || cv.getUint32(pos, true) !== 0x02014b50) throw new Error(ZIP_CORRUPT)
    const flags = cv.getUint16(pos + 8, true)
    const method = cv.getUint16(pos + 10, true)
    const dosTime = cv.getUint16(pos + 12, true)
    const dosDate = cv.getUint16(pos + 14, true)
    let compSize = cv.getUint32(pos + 20, true)
    let rawSize = cv.getUint32(pos + 24, true)
    const nameLen = cv.getUint16(pos + 28, true)
    const extraLen = cv.getUint16(pos + 30, true)
    const cmtLen = cv.getUint16(pos + 32, true)
    const next = pos + 46 + nameLen + extraLen + cmtLen
    if (next > cd.length && cdPartial) {
      scanLimited = true
      break
    }
    if (next > cd.length) throw new Error(ZIP_CORRUPT)
    const nameBytes = cd.subarray(pos + 46, pos + 46 + nameLen)
    const extra = cd.subarray(pos + 46 + nameLen, pos + 46 + nameLen + extraLen)
    const ev = view(extra)

    let name: string | null = null
    let mtimeMs = 0
    let nameCrc: number | null = null // 항목당 한 번만 계산한다(0x7075 필드가 여러 개일 수 있다)
    for (let e = 0; e + 4 <= extra.length; ) {
      const id = ev.getUint16(e, true)
      const len = ev.getUint16(e + 2, true)
      const data = extra.subarray(e + 4, Math.min(extra.length, e + 4 + len))
      const dv = view(data)
      if (id === 0x0001) {
        // ZIP64: 해당 필드가 0xFFFFFFFF 로 포화된 것만 순서대로(원본 → 압축) 들어 있다
        let o = 0
        if (rawSize === 0xffffffff && o + 8 <= data.length) {
          rawSize = safeNumber(dv.getBigUint64(o, true))
          o += 8
        }
        if (compSize === 0xffffffff && o + 8 <= data.length) compSize = safeNumber(dv.getBigUint64(o, true))
      } else if (id === 0x5455 && data.length >= 5 && (data[0] & 1) !== 0) {
        // Extended Timestamp: UTC 초 단위 수정 시각. DOS 시각(로컬·2초 단위)보다 정확하다
        mtimeMs = dv.getUint32(1, true) * 1000
      } else if (id === 0x7075 && data.length > 5 && data[0] === 1 && dv.getUint32(1, true) === (nameCrc ??= crc32(nameBytes))) {
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
    ...(scanLimited ? { scanLimited } : {}),
    totalSize,
    compressedSize,
    encrypted,
    ...(comment ? { comment } : {})
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
