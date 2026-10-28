// 폴더/압축 파일 목록 플러그인이 함께 쓰는 아이콘 조각.

export const FOLDER_ICON = `<svg viewBox="0 0 64 64" width="20" height="20" aria-hidden="true" focusable="false">
<path d="M6 14a3 3 0 0 1 3-3h15l6 6h25a3 3 0 0 1 3 3v31a3 3 0 0 1-3 3H9a3 3 0 0 1-3-3z" fill="var(--icon-folder)" stroke="var(--icon-edge)" stroke-width="3" stroke-linejoin="round"/>
</svg>`

/** "a.png" → "PNG" (최대 4자). 확장자가 없으면 "" */
export function extOf(name: string): string {
  const i = name.lastIndexOf('.')
  return i > 0 && i < name.length - 1 ? name.slice(i + 1).toUpperCase().slice(0, 4) : ''
}
