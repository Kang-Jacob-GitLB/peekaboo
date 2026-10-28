import type { FileInfo, PluginEntry, PreviewPlugin } from './types'

type PluginModule = { create: () => PreviewPlugin }

/** native 래퍼 모듈도 선택될 때만 로드한다. */
const nativeFirst = (fallback?: () => Promise<PluginModule>) =>
  import('./native').then((m) => m.nativeFirst(fallback))

const IMAGE = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'svg', 'bmp', 'ico', 'avif']
// 실제 재생 가능 여부는 Chromium 코덱 지원에 달려 있고, 못 하면 폴백 카드로 표시된다.
const MEDIA = [
  'mp4', 'webm', 'mkv', 'mov', 'm4v',
  'mp3', 'm4a', 'aac', 'wav', 'ogg', 'oga', 'opus', 'flac',
]
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
  { id: 'pdf', extensions: ['pdf'], load: () => import('./pdf') },
  // Office 문서: Windows 미리보기 핸들러(Office 설치 시 원본과 같은 모양) 우선, 없으면 자체 간이 렌더러
  { id: 'docx', extensions: ['docx'], load: () => nativeFirst(() => import('./docx')) },
  { id: 'xlsx', extensions: ['xlsx', 'xlsm', 'xls'], load: () => nativeFirst(() => import('./xlsx')) },
  { id: 'pptx', extensions: ['pptx'], load: () => nativeFirst(() => import('./pptx')) },
  // 구형 Office 형식은 자체 렌더러가 없어 핸들러가 없으면 폴백 카드로 표시
  { id: 'office-legacy', extensions: ['doc', 'ppt', 'rtf'], load: () => nativeFirst() },
  { id: 'media', extensions: MEDIA, load: () => import('./media') },
  // zip 계열(epub 포함)은 중앙 디렉터리만 읽어 파일 목록을 보여 준다. 7z/rar/tar 등은 폴백 카드
  { id: 'archive', extensions: ['zip', 'jar', 'apk', 'aab', 'vsix', 'nupkg', 'whl', 'xpi', 'epub'], load: () => import('./archive') },
  { id: 'folder', extensions: [], matchDirectory: true, load: () => import('./folder') },
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
