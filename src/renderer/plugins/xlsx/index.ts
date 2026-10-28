import * as XLSX from 'xlsx'
import type { PreviewPlugin, RenderContext } from '../types'
import { readBinary } from '../read-binary'

const MAX_ROWS = 1000
const MAX_COLS = 100

export function create(): PreviewPlugin {
  return {
    async render(ctx: RenderContext) {
      const { container, signal, setSubtitle } = ctx
      const data = await readBinary(ctx)
      if (signal.aborted) return

      // cellFormula 는 읽지 않는다 — 캐시된 계산 값(w/v)만 표시한다.
      const wb = XLSX.read(data, { type: 'array', cellDates: true, cellFormula: false, cellHTML: false })
      if (!wb.SheetNames.length) throw new Error('시트가 없는 통합 문서입니다.')

      container.classList.add('xlsx-view')
      const scroll = document.createElement('div')
      scroll.className = 'xlsx-scroll'
      const notice = document.createElement('div')
      notice.className = 'notice'
      notice.hidden = true
      const tabs = document.createElement('div')
      tabs.className = 'xlsx-tabs'
      tabs.setAttribute('role', 'tablist')
      container.append(notice, scroll, tabs)

      const buttons: HTMLButtonElement[] = []
      const select = (idx: number) => {
        buttons.forEach((b, i) => {
          b.classList.toggle('active', i === idx)
          b.setAttribute('aria-selected', String(i === idx))
        })
        scroll.replaceChildren(buildTable(wb.Sheets[wb.SheetNames[idx]], notice))
        scroll.scrollTo(0, 0)
      }
      wb.SheetNames.forEach((name, i) => {
        const b = document.createElement('button')
        b.type = 'button'
        b.setAttribute('role', 'tab')
        b.textContent = name
        b.title = name
        b.addEventListener('click', () => select(i))
        buttons.push(b)
        tabs.append(b)
      })
      select(0)

      const n = wb.SheetNames.length
      setSubtitle(`${ctx.file.ext.toUpperCase()} · ${n}시트`)
    },
  }
}

/** 시트 → <table>. 셀 값은 항상 textContent 로 넣는다. */
function buildTable(sheet: XLSX.WorkSheet, notice: HTMLElement): HTMLTableElement {
  const table = document.createElement('table')
  table.className = 'xlsx-table'
  const ref = sheet['!ref']
  if (!ref) {
    notice.hidden = true
    const td = table.insertRow().insertCell()
    td.textContent = '(빈 시트)'
    td.className = 'xlsx-empty'
    return table
  }
  const range = XLSX.utils.decode_range(ref)
  const rows = range.e.r - range.s.r + 1
  const cols = range.e.c - range.s.c + 1
  const lastRow = range.s.r + Math.min(rows, MAX_ROWS) - 1
  const lastCol = range.s.c + Math.min(cols, MAX_COLS) - 1

  // 병합 셀: 시작 셀에만 span 을 주고 나머지는 건너뛴다.
  const spans = new Map<string, { rs: number; cs: number }>()
  const covered = new Set<string>()
  for (const m of sheet['!merges'] ?? []) {
    spans.set(`${m.s.r},${m.s.c}`, { rs: m.e.r - m.s.r + 1, cs: m.e.c - m.s.c + 1 })
    for (let r = m.s.r; r <= m.e.r; r++)
      for (let c = m.s.c; c <= m.e.c; c++) if (r !== m.s.r || c !== m.s.c) covered.add(`${r},${c}`)
  }

  const head = table.createTHead().insertRow()
  head.insertCell().className = 'xlsx-corner'
  for (let c = range.s.c; c <= lastCol; c++) {
    const th = document.createElement('th')
    th.textContent = XLSX.utils.encode_col(c)
    head.append(th)
  }
  const body = table.createTBody()
  for (let r = range.s.r; r <= lastRow; r++) {
    const tr = body.insertRow()
    const rh = document.createElement('th')
    rh.textContent = String(r + 1)
    tr.append(rh)
    for (let c = range.s.c; c <= lastCol; c++) {
      if (covered.has(`${r},${c}`)) continue
      const td = tr.insertCell()
      const cell = sheet[XLSX.utils.encode_cell({ r, c })]
      if (cell) {
        // w = 서식이 적용된 표시 문자열(수식은 계산된 값)
        td.textContent = cell.w ?? (cell.v == null ? '' : String(cell.v))
        if (cell.t === 'n') td.className = 'num'
      }
      const sp = spans.get(`${r},${c}`)
      if (sp) {
        td.rowSpan = sp.rs
        td.colSpan = sp.cs
      }
    }
  }

  if (rows > MAX_ROWS || cols > MAX_COLS) {
    notice.hidden = false
    notice.textContent = `일부만 표시 중 — 전체 ${rows.toLocaleString()}행 × ${cols.toLocaleString()}열 중 최대 ${MAX_ROWS.toLocaleString()}행 × ${MAX_COLS}열`
  } else {
    notice.hidden = true
  }
  return table
}
