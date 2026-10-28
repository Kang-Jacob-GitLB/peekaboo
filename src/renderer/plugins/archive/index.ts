import type { ArchiveEntry } from '../../../shared/preview-api'
import type { PreviewPlugin, RenderContext } from '../types'
import { formatDate, formatSize } from '../format'
import { FOLDER_ICON, extOf } from '../file-icons'

const MAX_ENTRIES = 5000

const LOCK_ICON = `<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" focusable="false">
<rect x="3" y="7" width="10" height="7" rx="1.5" fill="currentColor"/>
<path d="M5 7V5a3 3 0 0 1 6 0v2" fill="none" stroke="currentColor" stroke-width="1.5"/>
</svg>`

const collator = new Intl.Collator('ko', { numeric: true, sensitivity: 'base' })

/** 경로로 합성한 트리 노드. 중간 디렉터리 항목이 압축 파일에 없어도 만들어진다. */
interface TreeNode {
  name: string
  /** 루트부터의 경로(펼침 상태 키) */
  key: string
  isDirectory: boolean
  entry?: ArchiveEntry
  children: Map<string, TreeNode>
  /** 정렬된 자식 — 처음 펼칠 때 한 번만 만든다 */
  sorted?: TreeNode[]
}

function newNode(name: string, key: string, isDirectory: boolean): TreeNode {
  return { name, key, isDirectory, children: new Map() }
}

/** 트리 깊이·노드 수 상한 — 조작된 zip 의 아주 깊은 경로로 렌더러 메모리를 고갈시키지 못하게 */
const MAX_DEPTH = 64
const MAX_NODES = 50_000

function buildTree(entries: ArchiveEntry[]): TreeNode {
  const root = newNode('', '', true)
  let nodes = 0
  for (const e of entries) {
    // "./a", "/a", "a//b" 같은 경로 정리. ".." 은 표시용 트리에서도 의미가 없으므로 버린다.
    let parts = e.path.split('/').filter((p) => p !== '' && p !== '.' && p !== '..')
    if (parts.length === 0) continue
    if (parts.length > MAX_DEPTH) parts = [...parts.slice(0, MAX_DEPTH - 1), parts.slice(MAX_DEPTH - 1).join('/')]
    nodes += parts.length
    if (nodes > MAX_NODES) break
    let cur = root
    parts.forEach((part, i) => {
      const last = i === parts.length - 1
      const isDir = !last || e.isDirectory
      let child = cur.children.get(part)
      if (!child) {
        child = newNode(part, cur.key ? `${cur.key}/${part}` : part, isDir)
        cur.children.set(part, child)
      } else if (isDir) {
        child.isDirectory = true
      }
      if (last) child.entry = e
      cur = child
    })
  }
  return root
}

/** 폴더 먼저, 그다음 이름순(숫자는 자연 정렬). */
function sortedChildren(node: TreeNode): TreeNode[] {
  node.sorted ??= [...node.children.values()].sort(
    (a, b) => Number(b.isDirectory) - Number(a.isDirectory) || collator.compare(a.name, b.name),
  )
  return node.sorted
}

function cell(className: string, text = ''): HTMLElement {
  const el = document.createElement('div')
  el.className = className
  el.textContent = text
  return el
}

export function create(): PreviewPlugin {
  return {
    async render({ file, container, signal, host, setSubtitle }: RenderContext) {
      const listing = await host.listArchive(file.path, MAX_ENTRIES)
      if (signal.aborted) return
      container.classList.add('archive-view')

      const fileCount = listing.entries.filter((e) => !e.isDirectory).length
      const parts = [file.ext.toUpperCase() || 'ZIP']
      parts.push(listing.truncated ? `항목 ${listing.totalEntries.toLocaleString('ko-KR')}개` : `파일 ${fileCount.toLocaleString('ko-KR')}개`)
      if (listing.totalSize > 0) {
        const ratio = Math.round((listing.compressedSize / listing.totalSize) * 100)
        parts.push(`원본 ${formatSize(listing.totalSize)}(압축 ${formatSize(listing.compressedSize)}, ${ratio}%)`)
      }
      setSubtitle(parts.join(' · '))

      const notices: string[] = []
      if (listing.truncated)
        notices.push(`항목이 많아 처음 ${listing.entries.length.toLocaleString('ko-KR')}개만 표시합니다. 크기 합계도 이 항목들만의 값입니다.`)
      if (listing.encrypted) notices.push('암호화된 항목이 있습니다. 목록만 볼 수 있고 내용은 열 수 없습니다.')
      if (listing.comment) notices.push(`압축 파일 주석: ${listing.comment}`)
      for (const text of notices) {
        const notice = document.createElement('div')
        notice.className = 'notice'
        notice.textContent = text
        container.append(notice)
      }

      if (listing.entries.length === 0) {
        const empty = document.createElement('div')
        empty.className = 'dir-empty'
        empty.textContent = '비어 있는 압축 파일입니다'
        container.append(empty)
        return
      }

      const root = buildTree(listing.entries)
      // 기본 펼침: 최상위 폴더(= 루트 아래 1단계 내용이 보이도록)
      const expanded = new Set<string>()
      for (const child of root.children.values()) if (child.isDirectory) expanded.add(child.key)

      const scroll = document.createElement('div')
      scroll.className = 'arc-scroll'
      container.append(scroll)

      // 펼쳐진 노드의 자식만 DOM 으로 만든다. 접기/펼치기 때는 목록을 다시 그리되 스크롤 위치는 유지한다.
      const draw = (): void => {
        const top = scroll.scrollTop
        const frag = document.createDocumentFragment()
        const walk = (node: TreeNode, depth: number): void => {
          for (const child of sortedChildren(node)) {
            const open = child.isDirectory && expanded.has(child.key)
            frag.append(row(child, depth, open))
            if (open) walk(child, depth + 1)
          }
        }
        walk(root, 0)
        scroll.replaceChildren(frag)
        scroll.scrollTop = top
      }

      const row = (node: TreeNode, depth: number, open: boolean): HTMLElement => {
        const el = document.createElement('div')
        el.className = 'arc-row'
        const label = document.createElement('div')
        label.className = 'arc-label'
        label.style.paddingLeft = `${depth * 18 + 10}px`

        const twisty = document.createElement('span')
        twisty.className = 'arc-twisty'
        if (node.isDirectory) twisty.textContent = open ? '▾' : '▸'
        label.append(twisty)

        const icon = document.createElement('span')
        icon.className = 'arc-icon'
        if (node.isDirectory) {
          icon.innerHTML = FOLDER_ICON
        } else {
          const badge = document.createElement('span')
          badge.className = 'dir-ext'
          badge.textContent = extOf(node.name)
          icon.append(badge)
        }
        label.append(icon)

        const name = document.createElement('span')
        name.className = 'arc-name'
        // 제어 문자·양방향 재배치 문자(U+202E 등)는 "x\u202Efdp.exe" 를 "xexe.pdf" 처럼 보이게 하므로 치환
        const shown = node.name.replace(/[\u0000-\u001f\u007f\u200e\u200f\u202a-\u202e\u2066-\u2069]/g, '\uFFFD')
        name.textContent = shown
        name.title = shown
        label.append(name)

        const e = node.entry
        const lock = cell('arc-lock')
        if (e?.encrypted) {
          lock.innerHTML = LOCK_ICON
          lock.title = '암호화됨'
        }
        el.append(
          label,
          cell('dir-size', e && !node.isDirectory ? formatSize(e.size) : ''),
          cell('dir-date', e && e.mtimeMs > 0 ? formatDate(e.mtimeMs) : ''),
          lock,
        )

        if (node.isDirectory) {
          el.classList.add('arc-dir')
          el.addEventListener('click', () => {
            if (expanded.has(node.key)) expanded.delete(node.key)
            else expanded.add(node.key)
            draw()
          })
        }
        return el
      }

      draw()
    },
  }
}
