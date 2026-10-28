import type { FileInfo, PluginEntry } from './types'

// 포맷별 플러그인 목록. 순서 = 우선순위. load() 는 선택될 때만 호출된다.
export const plugins: PluginEntry[] = [
  // 1단계: image / text / markdown — 다음 커밋에서 추가
]

export function findPlugin(file: FileInfo): PluginEntry | undefined {
  if (file.isDirectory) return plugins.find((p) => p.matchDirectory)
  return (
    plugins.find((p) => p.extensions.includes(file.ext)) ?? plugins.find((p) => p.extensions.includes('*'))
  )
}
