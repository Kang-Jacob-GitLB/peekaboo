import { findPlugin } from './plugins/registry'
import { formatSize } from './plugins/format'
import type { PreviewPlugin } from './plugins/types'
import type { ShowPayload } from '../shared/preview-api'

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

  const ctx = {
    file,
    container,
    signal: abort.signal,
    host,
    setSubtitle: (t: string) => {
      if (!stale()) elSub.textContent = t
    },
  }

  try {
    const entry = findPlugin(file)
    if (!entry) throw new Error('사용 가능한 플러그인이 없습니다.')
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
    const { renderFallback } = await import('./plugins/fallback')
    if (stale()) return
    renderFallback(container, file, `미리보기를 만들지 못했습니다: ${errorText(err)}`)
  } finally {
    clearTimeout(timer)
    loader?.remove()
  }
  if (stale()) return
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
  void nextPaint().then(() => {
    if (current.seq === e.seq) host.rendered(e.seq)
  })
})

host.onHide(() => teardown())

/** ipcRenderer.invoke 가 붙이는 "Error invoking remote method '…': Error: " 접두어를 떼고 사람이 읽을 메시지만 */
function errorText(err: unknown): string {
  const msg = err instanceof Error ? err.message : String(err)
  return msg.replace(/^Error invoking remote method '[^']*': (?:\w*Error: )?/, '')
}
