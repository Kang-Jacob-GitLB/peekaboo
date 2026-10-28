import type { RenderContext } from './types'

/** 오피스/PDF 플러그인이 메모리에 통째로 올릴 수 있는 최대 파일 크기 */
const MAX_BYTES = 100 * 1024 * 1024

/** peek:// 로 파일 전체를 읽는다. 상한 초과면 읽기 전에 throw 한다. */
export async function readBinary({ file, host, signal }: RenderContext): Promise<ArrayBuffer> {
  if (file.size > MAX_BYTES) throw new Error('파일이 너무 커서(100MB 초과) 미리볼 수 없습니다.')
  const res = await fetch(host.fileUrl(file.path), { signal })
  if (!res.ok) throw new Error('파일을 읽을 수 없습니다.')
  return res.arrayBuffer()
}
