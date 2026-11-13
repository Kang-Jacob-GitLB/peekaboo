// 압축 파일 목록의 진입점. 확장자로 tar 계열과 zip 계열을 고른다.

import type { ArchiveListing } from '../../../shared/preview-api'
import { ZIP_CORRUPT, decodeZipName } from './bytes.ts'
import type { RangeReader } from './range-reader.ts'
import { GZ_MAX_SCAN_BYTES, TAR_CORRUPT, isGzip, isTarHeader, listTar } from './tar.ts'
import { ZIP_MAX_CD_BYTES, listZip } from './zip.ts'

/** 테스트가 작은 값으로 상한 동작을 확인할 수 있게 둔 조정점. 앱은 기본값만 쓴다 */
export interface ArchiveCaps {
  zipMaxCdBytes: number
  gzMaxScanBytes: number
}

export const DEFAULT_CAPS: ArchiveCaps = { zipMaxCdBytes: ZIP_MAX_CD_BYTES, gzMaxScanBytes: GZ_MAX_SCAN_BYTES }

/**
 * 압축을 풀지 않고 목록만 만든다.
 * - tar·tgz·gz 확장자: 내용이 gzip 이면 tar.gz, 아니면 평문 tar 로 읽는다 (tar.ts)
 * - 그 밖(zip 계열): 끝부분(EOCD) → 중앙 디렉터리만 읽는다(ZIP64 지원). 항목은 max 개까지만 파싱한다.
 * 형식을 확장자 범주 안에서만 고르는 이유: 앞에 tar 헤더를 붙인 zip(규격상 유효)을 tar 로 읽으면,
 * 탐색기·7-Zip 이 실제로 푸는 항목과 다른 위장 목록을 보여 주게 된다.
 */
export async function listArchive(reader: RangeReader, path: string, max: number, caps: ArchiveCaps = DEFAULT_CAPS): Promise<ArchiveListing> {
  const limit = Number.isFinite(max) ? Math.max(0, Math.min(Math.floor(max), 100_000)) : 0
  const size = await reader.size()
  if (/\.(tar|tgz|gz)$/i.test(path)) {
    const head = await reader.read(0, Math.min(size, 512))
    if (!isGzip(head) && !isTarHeader(head)) throw new Error(TAR_CORRUPT)
    return await listTar(reader, size, isGzip(head), limit, (b) => decodeZipName(b, false), caps.gzMaxScanBytes)
  }
  if (size < 22) throw new Error(ZIP_CORRUPT)
  return await listZip(reader, size, limit, caps.zipMaxCdBytes)
}
