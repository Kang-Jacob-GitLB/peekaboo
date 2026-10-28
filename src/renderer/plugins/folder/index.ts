import type { DirEntry } from '../../../shared/preview-api'
import type { PreviewPlugin, RenderContext } from '../types'
import { formatDate, formatSize } from '../format'
import { FOLDER_ICON, extOf } from '../file-icons'

const MAX_ENTRIES = 500

const collator = new Intl.Collator('ko', { numeric: true, sensitivity: 'base' })

/** 폴더 먼저, 그다음 이름순(숫자는 자연 정렬). */
function sortEntries(entries: DirEntry[]): DirEntry[] {
  return [...entries].sort(
    (a, b) => Number(b.isDirectory) - Number(a.isDirectory) || collator.compare(a.name, b.name),
  )
}

function row(e: DirEntry): HTMLElement {
  const tr = document.createElement('tr')
  const iconCell = document.createElement('td')
  iconCell.className = 'dir-icon'
  if (e.isDirectory) {
    iconCell.innerHTML = FOLDER_ICON
  } else {
    // 파일은 확장자 배지로 종류를 구분한다.
    const badge = document.createElement('span')
    badge.className = 'dir-ext'
    badge.textContent = extOf(e.name)
    iconCell.append(badge)
  }
  const name = document.createElement('td')
  name.className = 'dir-name'
  name.textContent = e.name
  name.title = e.name
  const size = document.createElement('td')
  size.className = 'dir-size'
  size.textContent = e.isDirectory ? '' : formatSize(e.size)
  const date = document.createElement('td')
  date.className = 'dir-date'
  date.textContent = formatDate(e.mtimeMs)
  tr.append(iconCell, name, size, date)
  return tr
}

export function create(): PreviewPlugin {
  return {
    async render({ file, container, signal, host, setSubtitle }: RenderContext) {
      const { entries, truncated } = await host.listDir(file.path, MAX_ENTRIES)
      if (signal.aborted) return
      container.classList.add('folder-view')

      const sorted = sortEntries(entries)
      const total = entries.reduce((sum, e) => (e.isDirectory ? sum : sum + e.size), 0)
      const count = `${entries.length}${truncated ? '개 이상' : '개'}`
      setSubtitle(`폴더 · 항목 ${count} · 합계 ${formatSize(total)}`)

      if (sorted.length === 0) {
        const empty = document.createElement('div')
        empty.className = 'dir-empty'
        empty.textContent = '비어 있는 폴더입니다'
        container.append(empty)
        return
      }

      if (truncated) {
        const notice = document.createElement('div')
        notice.className = 'notice'
        notice.textContent = `항목이 많아 처음 ${entries.length}개만 표시합니다. 나머지는 탐색기에서 확인하세요.`
        container.append(notice)
      }

      const scroll = document.createElement('div')
      scroll.className = 'dir-scroll'
      const table = document.createElement('table')
      table.className = 'dir-table'
      const body = document.createElement('tbody')
      body.append(...sorted.map(row))
      table.append(body)
      scroll.append(table)
      container.append(scroll)
    },
  }
}
