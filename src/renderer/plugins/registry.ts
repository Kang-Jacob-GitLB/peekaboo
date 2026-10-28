import type { FileInfo, PluginEntry } from './types'

const IMAGE = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'ico', 'avif']
const TEXT = [
  'txt', 'log', 'json', 'js', 'ts', 'tsx', 'jsx', 'mjs', 'cjs', 'css', 'scss', 'html', 'xml',
  'yaml', 'yml', 'toml', 'ini', 'cfg', 'conf', 'csv', 'tsv', 'py', 'rb', 'go', 'rs', 'java', 'kt',
  'c', 'h', 'cpp', 'hpp', 'cc', 'cs', 'swift', 'php', 'sh', 'bash', 'ps1', 'bat', 'cmd', 'sql',
  'lua', 'r', 'dart', 'vue', 'svelte', 'gradle', 'properties', 'diff', 'patch', 'env',
  // 확장자가 없는 파일은 소문자 파일명 전체로 매칭된다(findPlugin 참고).
  'gitignore', '.gitignore', '.gitattributes', '.editorconfig', '.env', 'dockerfile', 'makefile',
]

// 포맷별 플러그인 목록. 순서 = 우선순위. load() 는 선택될 때만 호출된다.
export const plugins: PluginEntry[] = [
  { id: 'image', extensions: IMAGE, load: () => import('./image') },
  { id: 'markdown', extensions: ['md', 'markdown', 'mdx'], load: () => import('./markdown') },
  { id: 'text', extensions: TEXT, load: () => import('./text') },
  { id: 'fallback', extensions: ['*'], matchDirectory: true, load: () => import('./fallback') },
]

export function findPlugin(file: FileInfo): PluginEntry | undefined {
  if (file.isDirectory) return plugins.find((p) => p.matchDirectory)
  // 확장자 → (확장자 없는 파일/도트파일을 위해) 소문자 파일명 → 폴백 순
  const name = file.name.toLowerCase()
  return (
    plugins.find((p) => p.extensions.includes(file.ext)) ??
    plugins.find((p) => p.extensions.includes(name)) ??
    plugins.find((p) => p.extensions.includes('*'))
  )
}
