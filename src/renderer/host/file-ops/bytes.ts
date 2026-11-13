// 바이트 처리 도구. main 의 Buffer·node:zlib 를 대신한다(렌더러에는 Node API 가 없다).
// 상대 import 에 .ts 를 붙이는 것은 node --test 가 이 파일들을 직접 불러오기 때문이다.

export function view(b: Uint8Array): DataView {
  return new DataView(b.buffer, b.byteOffset, b.byteLength)
}

export function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length)
  out.set(a, 0)
  out.set(b, a.length)
  return out
}

/** Buffer#indexOf(byte, from) */
export function indexOfByte(b: Uint8Array, byte: number, from = 0): number {
  for (let i = from; i < b.length; i++) if (b[i] === byte) return i
  return -1
}

/** Buffer#toString('latin1'): 바이트 하나가 U+0000~U+00FF 한 글자. TextDecoder('latin1') 은 windows-1252 라 다르다 */
export function latin1(b: Uint8Array, start = 0, end = b.length): string {
  let out = ''
  for (let i = start; i < end; i += 8192) out += String.fromCharCode(...b.subarray(i, Math.min(end, i + 8192)))
  return out
}

/** Buffer#toString('utf8') 와 같이 앞의 BOM 을 지우지 않는다 */
export function utf8(b: Uint8Array): string {
  return new TextDecoder('utf-8', { ignoreBOM: true }).decode(b)
}

const CRC_TABLE = (() => {
  const t = new Uint32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c
  }
  return t
})()

/** node:zlib 의 crc32 와 같은 CRC-32(IEEE). 부호 없는 32비트 정수 */
export function crc32(b: Uint8Array): number {
  let c = 0xffffffff
  for (let i = 0; i < b.length; i++) c = CRC_TABLE[(c ^ b[i]) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

export const ZIP_CORRUPT = '손상되었거나 zip 형식이 아닌 파일입니다'

export function safeNumber(v: bigint): number {
  if (v > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error(ZIP_CORRUPT)
  return Number(v)
}

/** UTF-8 플래그 → UTF-8. 아니면 유효한 UTF-8 이면 UTF-8, 아니면 CP949(EUC-KR), 그것도 안 되면 latin1. zip 이름·tar 이름 공용 */
export function decodeZipName(bytes: Uint8Array, utf8Flag: boolean): string {
  if (utf8Flag) return new TextDecoder('utf-8').decode(bytes)
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes)
  } catch {
    try {
      return new TextDecoder('euc-kr', { fatal: true }).decode(bytes)
    } catch {
      return latin1(bytes)
    }
  }
}
