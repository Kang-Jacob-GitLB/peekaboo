import { open, readdir, stat } from 'node:fs/promises'
import { basename, extname, join } from 'node:path'
import type { DirEntry, FileInfo, TextResult } from '../shared/preview-api'

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
