// 텍스트 앞부분 읽기·디코딩. main 의 file-service.ts(readTextHead, decode)를 렌더러로 옮긴 것이다.

import type { TextResult } from '../../../shared/preview-api'
import type { RangeReader } from './range-reader.ts'

/** 한 번에 읽는 최대 바이트. 호출부가 더 큰 값을 줘도 여기서 자른다 */
const MAX_READ_BYTES = 16 * 1024 * 1024

/** 앞에서 maxBytes 만 읽어 텍스트로 디코딩. BOM → UTF-8 검증 → 실패 시 EUC-KR(CP949) 순으로 판단. */
export async function readTextHead(reader: RangeReader, maxBytes: number): Promise<TextResult> {
  const limit = Number.isFinite(maxBytes) ? Math.max(0, Math.min(Math.floor(maxBytes), MAX_READ_BYTES)) : 0
  const bytes = limit > 0 ? await reader.read(0, limit) : new Uint8Array(0)
  const size = await reader.size()
  const truncated = size > bytes.length
  const { text, encoding } = decode(bytes, truncated)
  return { text, truncated, encoding }
}

function decode(bytes: Uint8Array, truncated: boolean): { text: string; encoding: string } {
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
