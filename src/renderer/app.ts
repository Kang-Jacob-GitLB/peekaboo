import { findPlugin } from './plugins/registry'
import { formatSize } from './plugins/format'
import type { PreviewPlugin } from './plugins/types'
import type { ContentSize, ShowPayload } from '../shared/preview-api'

const host = window.peekaboo
const LOADING_DELAY_MS = 120

const app = document.getElementById('app')!
app.innerHTML = `
<header>
  <span class="h-name"></span><span class="h-size"></span><span class="h-badge" hidden></span>
  <span class="h-sub"></span>
</header>
<main></main>`
const $ = (sel: string) => app.querySelector<HTMLElement>(sel)!
const elName = $('.h-name')
const elSize = $('.h-size')
const elBadge = $('.h-badge')
const elSub = $('.h-sub')
const main = $('main')

let current = { seq: -1, abort: null as AbortController | null, plugin: null as PreviewPlugin | null }

function teardown(): void {
  current.abort?.abort()
  try {
    current.plugin?.dispose?.()
  } catch (e) {
    console.error('dispose failed', e)
  }
  current.abort = null
  current.plugin = null
  main.replaceChildren()
  // 내용을 지우면 선택도 사라진다(지운 iframe 의 문서는 selectionchange 를 더 보내지 않는다)
  lastSelectionDoc = null
  reportSelection()
}

function nextPaint(): Promise<void> {
  return new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())))
}

function setHeader(p: ShowPayload): void {
  elName.textContent = p.file.name
  elName.title = p.file.path
  elSize.textContent = p.file.isDirectory ? '폴더' : formatSize(p.file.size)
  elBadge.hidden = p.selectionCount <= 1
  elBadge.textContent = `선택 ${p.selectionCount}개`
  elSub.textContent = p.file.isDirectory ? '' : p.file.ext.toUpperCase()
}

async function show(payload: ShowPayload): Promise<void> {
  teardown()
  const { seq, file } = payload
  const abort = new AbortController()
  current = { seq, abort, plugin: null }
  const stale = () => current.seq !== seq || abort.signal.aborted
  setHeader(payload)

  const container = document.createElement('div')
  container.className = 'content'
  main.append(container)

  let loader = null as HTMLElement | null
  const timer = setTimeout(() => {
    if (stale()) return
    loader = document.createElement('div')
    loader.className = 'loading'
    loader.setAttribute('role', 'status')
    loader.innerHTML = '<span class="spinner"></span><span>불러오는 중…</span>'
    main.append(loader)
  }, LOADING_DELAY_MS)

  // 창 크기: 플러그인이 알려 준 내용 크기 → registry 의 페이지 비율 → 기본 크기 순
  let fitted = false
  const fit = (size: ContentSize | null) => {
    if (stale()) return
    fitted = size !== null
    host.fit(seq, size)
  }

  const ctx = {
    file,
    container,
    signal: abort.signal,
    host,
    setSubtitle: (t: string) => {
      if (!stale()) elSub.textContent = t
    },
    fitContent: (size: ContentSize) => fit(size),
  }

  try {
    const entry = findPlugin(file)
    if (!entry) throw new Error('사용 가능한 플러그인이 없습니다.')
    if (entry.aspect) fit({ width: entry.aspect[0], height: entry.aspect[1], grow: true })
    const mod = await entry.load()
    if (stale()) return
    const plugin = mod.create()
    current.plugin = plugin
    await plugin.render(ctx)
  } catch (err) {
    if (stale()) return
    console.error(err)
    container.className = 'content'
    container.replaceChildren()
    fitted = false
    const { renderFallback } = await import('./plugins/fallback')
    if (stale()) return
    renderFallback(container, file, `미리보기를 만들지 못했습니다: ${errorText(err)}`)
  } finally {
    clearTimeout(timer)
    loader?.remove()
  }
  if (stale()) return
  if (!fitted) fit(null)
  await nextPaint()
  if (!stale()) host.rendered(seq)
}

host.onShow((p) => void show(p))

host.onShowError((e) => {
  teardown()
  current = { seq: e.seq, abort: null, plugin: null }
  elName.textContent = e.path.split(/[\\/]/).pop() || e.path
  elName.title = e.path
  elSize.textContent = ''
  elBadge.hidden = true
  elSub.textContent = ''
  const view = document.createElement('div')
  view.className = 'content error-view'
  for (const [cls, text] of [
    ['err-title', '파일을 열 수 없습니다'],
    ['err-path', e.path],
    ['err-msg', e.message],
  ] as const) {
    const d = document.createElement('div')
    d.className = cls
    d.textContent = text
    view.append(d)
  }
  main.append(view)
  host.fit(e.seq, null)
  void nextPaint().then(() => {
    if (current.seq === e.seq) host.rendered(e.seq)
  })
})

host.onHide(() => teardown())

// ---- 텍스트 선택과 Ctrl+C 복사 ----
// 창이 포커스를 갖지 않아 Ctrl+C 는 탐색기로 간다. 선택이 있는 동안만 헬퍼가 Ctrl+C 를 가로채도록 선택 여부를 알리고,
// 가로챈 Ctrl+C 는 main 이 onCopyRequest 로 선택한 텍스트를 받아 클립보드에 쓴다.
// HTML 미리보기 iframe(allow-same-origin)의 문서는 선택을 따로 가지므로 iframe 이 로드될 때 같이 구독한다.
const selectionDocs = new Set<Document>()
let lastSelectionDoc: Document | null = null
let reportedSelected = false

function liveDocs(): Document[] {
  for (const d of selectionDocs) if (d !== document && !d.defaultView) selectionDocs.delete(d) // 제거된 iframe
  return [...selectionDocs]
}

function hasSelection(doc: Document): boolean {
  const sel = doc.getSelection()
  return !!sel && sel.rangeCount > 0 && !sel.isCollapsed
}

function reportSelection(doc?: Document): void {
  if (doc && hasSelection(doc)) lastSelectionDoc = doc
  const selected = liveDocs().some(hasSelection)
  if (selected === reportedSelected) return
  reportedSelected = selected
  host.textSelected(selected)
}

function watchSelection(doc: Document): void {
  if (selectionDocs.has(doc)) return
  selectionDocs.add(doc)
  doc.addEventListener('selectionchange', () => reportSelection(doc))
}

watchSelection(document)
// iframe 의 load 는 버블링하지 않으므로 캡처 단계에서 받는다
document.addEventListener(
  'load',
  (e) => {
    if (!(e.target instanceof HTMLIFrameElement)) return
    try {
      const doc = e.target.contentDocument
      if (doc) watchSelection(doc)
    } catch {
      // 다른 출처 문서는 읽을 수 없다
    }
  },
  true
)

host.onCopyRequest(() => {
  // 가장 최근에 선택한 문서를 먼저, 없으면 선택이 남아 있는 아무 문서
  const docs = liveDocs()
  const doc = lastSelectionDoc && docs.includes(lastSelectionDoc) && hasSelection(lastSelectionDoc)
    ? lastSelectionDoc
    : docs.find(hasSelection)
  return doc?.getSelection()?.toString() ?? ''
})

// 복사 알림(복사 자체는 main 이 한다)
let toastTimer: ReturnType<typeof setTimeout> | undefined
host.onCopied(() => {
  let toast = document.querySelector<HTMLElement>('.toast')
  if (!toast) {
    toast = document.createElement('div')
    toast.className = 'toast'
    toast.setAttribute('role', 'status')
    toast.textContent = '복사했습니다'
    document.body.append(toast)
  }
  toast.classList.add('show')
  clearTimeout(toastTimer)
  toastTimer = setTimeout(() => toast.classList.remove('show'), 1200)
})

/** ipcRenderer.invoke 가 붙이는 "Error invoking remote method '…': Error: " 접두어를 떼고 사람이 읽을 메시지만 */
function errorText(err: unknown): string {
  const msg = err instanceof Error ? err.message : String(err)
  return msg.replace(/^Error invoking remote method '[^']*': (?:\w*Error: )?/, '')
}
