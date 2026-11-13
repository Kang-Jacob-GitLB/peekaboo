import { readdir, stat } from 'node:fs/promises'
import { basename, extname, join } from 'node:path'
import type { DirEntry, FileInfo } from '../shared/preview-api'

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
