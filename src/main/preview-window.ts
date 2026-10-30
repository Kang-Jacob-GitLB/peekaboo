import { BrowserWindow, ipcMain, nativeTheme, screen } from 'electron'
import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { Channels } from '../shared/channels'
import type { ContentSize, ShowPayload } from '../shared/preview-api'
import { getFileInfo, listArchive, listDirectory, readTextHead } from './file-service'
import { createLogger } from './logger'
import type { NativePreviewBridge } from './native-preview'
import type { PixelRect } from '../shared/ipc-types'

const log = createLogger('preview')

// 내용 크기에 맞출 때: 창 테두리(1px×2)와 헤더(38px + 테두리)
const CHROME_W = 2
const CHROME_H = 40
// 작은 사진도 헤더(파일명·크기)가 읽히도록 하는 최소 내용 크기
const MIN_CONTENT_W = 420
const MIN_CONTENT_H = 240
// 내용에 맞출 때 작업 영역에서 차지할 수 있는 최대 비율
const MAX_AREA_RATIO = 0.85
// 처음 띄울 때 렌더러가 내용 크기를 알려 주기를 기다리는 최대 시간(기본 크기로 떴다가 바뀌며 튀지 않게)
const FIRST_FIT_WAIT_MS = 150
// 렌더러 로드가 이보다 오래 걸리면 경고를 남긴다(보안 제품·부하로 렌더러 프로세스 기동이 수십 초씩 걸리는 PC가 있다)
const SLOW_LOAD_MS = 2000

export interface PreviewRequest {
  path: string
  selectionCount: number
  /** 지연 측정 시작점(키 입력 수신 시각, performance.now()) */
  t0: number
}

/**
 * 미리보기 창 하나를 관리한다.
 * - 프레임 없음, 항상 위, 포커스를 가져가지 않음(focusable:false + showInactive) → 탐색기 포커스 유지
 * - 시작할 때 창을 숨긴 채 미리 로드해 둔다.
 * - 화면에 띄웠던 창은 닫은 뒤 새 창으로 바꾼다. 새 창도 숨긴 채 미리 로드하고, 로드가 끝난 뒤에야 교체한다
 *   (그 전에 다시 열면 숨겨 둔 기존 창을 바로 쓴다). 렌더러 기동이 수십 초씩 걸리는 PC 에서도 Space 가 기다리지 않게 하기 위함이다.
 *   keepWarmSeconds > 0 이면 교체를 그 시간만큼 미룬다.
 */
export class PreviewWindow {
  private win: BrowserWindow | null = null
  /** win 의 렌더러 로드 완료(성공이면 true). 실패해도 reject 하지 않는다. */
  private loaded: Promise<boolean> | null = null
  /** 교체용으로 미리 로드 중이거나 로드를 마친 창 */
  private spare: { win: BrowserWindow; loaded: Promise<boolean>; ready: boolean } | null = null
  private destroyTimer: NodeJS.Timeout | null = null
  private recycleTimer: NodeJS.Timeout | null = null
  private seq = 0
  private currentPath: string | null = null
  private pendingT0 = new Map<number, number>()
  private visible = false
  /** 진행 중인 show() 의 seq(창을 띄우기 전 파일 정보·렌더러 로드 대기 중). 0 이면 없음 */
  private openingSeq = 0
  /** 창을 띄우기 전에 받은 크기 요청(undefined = 아직 없음, null = 기본 크기) */
  private firstFit: ContentSize | null | undefined
  private firstFitWaiter: (() => void) | null = null

  constructor(
    private readonly keepWarmSeconds: () => number,
    private readonly native: NativePreviewBridge
  ) {
    this.registerIpc()
  }

  /** 보이는 중이거나 띄우는 중 */
  get isOpen(): boolean {
    return this.visible || this.openingSeq !== 0
  }

  /** peek:// 와 readText 가 허용하는 유일한 경로 */
  isAllowedPath(path: string): boolean {
    // 렌더러는 main 이 준 경로를 그대로 돌려준다. 대소문자 무시 비교는 JS 와 NTFS 의 대소문자 규칙이 달라
    // (예: U+212A KELVIN SIGN ↔ 'k') 같은 폴더의 다른 파일을 지정할 여지가 있으므로 정확히 일치만 허용한다.
    return typeof path === 'string' && this.currentPath !== null && path === this.currentPath
  }

  async show(req: PreviewRequest): Promise<void> {
    const seq = ++this.seq
    this.openingSeq = seq
    this.pendingT0.set(seq, req.t0)
    this.currentPath = req.path

    const win = this.ensureWindow('show')
    const loaded = this.loaded
    let payload: ShowPayload | null = null
    let error: string | null = null
    try {
      payload = { seq, file: await getFileInfo(req.path), selectionCount: req.selectionCount }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
    await loaded
    if (seq !== this.seq || win.isDestroyed()) return // 그 사이 다른 파일이 요청됐거나 hide() 로 취소됨

    if (!this.visible) this.firstFit = undefined
    if (payload) win.webContents.send(Channels.show, payload)
    else win.webContents.send(Channels.showError, { seq, path: req.path, message: error })

    if (!this.visible) {
      // 기다리는 동안에도 openingSeq 를 유지해 isOpen 이 true 로 남게 한다(그 사이 Esc·앱 전환·Space 로 취소 가능)
      await this.waitFirstFit()
      if (seq !== this.seq || win.isDestroyed()) return
      const area = cursorWorkArea()
      if (this.firstFit) this.fit(win, this.firstFit, area)
      else this.position(win, area)
      win.showInactive()
      this.visible = true
      log.info(`shown in ${(performance.now() - req.t0).toFixed(0)}ms since key`)
    }
    if (this.openingSeq === seq) this.openingSeq = 0
  }

  private waitFirstFit(): Promise<void> {
    if (this.firstFit !== undefined) return Promise.resolve()
    return new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer)
        if (this.firstFitWaiter === done) this.firstFitWaiter = null
        resolve()
      }
      const timer = setTimeout(done, FIRST_FIT_WAIT_MS)
      this.firstFitWaiter = done
    })
  }

  hide(): void {
    const wasOpening = this.openingSeq !== 0
    const wasVisible = this.visible
    // 띄우는 중이면 취소한다: seq 를 올리면 대기 중인 show() 가 창을 띄우지 않고 끝난다.
    // (취소하지 않으면 Space 를 빠르게 두 번 누르거나 그 사이 다른 앱으로 전환했을 때 창이 뒤늦게 떠서 남는다)
    this.seq++
    this.openingSeq = 0
    if (!this.win || (!wasVisible && !wasOpening)) return
    this.visible = false
    this.currentPath = null
    this.native.closeAll('hidden')
    this.win.webContents.send(Channels.hide)
    this.win.hide()
    // 띄우기 전에 취소된 창(렌더러 로드 대기 중 등)은 그대로 다음에 쓴다. 바꾸면 그때까지의 로드를 버리게 된다.
    if (!wasVisible) return
    if (this.spare?.ready) {
      this.swapToSpare()
      return
    }
    const keep = this.keepWarmSeconds()
    if (keep <= 0) this.scheduleRecycle()
    else this.destroyTimer = setTimeout(() => this.scheduleRecycle(), keep * 1000)
  }

  /** 시작할 때 창을 숨긴 채 미리 만들어 로드해 둔다. 이미 있으면 아무것도 하지 않는다. */
  prewarm(): void {
    if (!this.win || this.win.isDestroyed()) this.ensureWindow('prewarm')
  }

  /** 교체용 새 창을 준비한다. 키 처리 경로를 막지 않도록 창 생성은 다음 틱으로 미룬다. */
  private scheduleRecycle(): void {
    this.destroyTimer = null
    if (this.spare || this.recycleTimer) return
    this.recycleTimer = setTimeout(() => {
      this.recycleTimer = null
      const spare = { ...this.createWindow('spare'), ready: false }
      this.spare = spare
      void spare.loaded.then((ok) => {
        if (this.spare !== spare) return
        if (!ok) {
          // 로드에 실패한 창으로는 바꾸지 않는다. 다음에 닫을 때 다시 준비한다.
          this.spare = null
          if (!spare.win.isDestroyed()) spare.win.destroy()
          return
        }
        spare.ready = true
        this.swapToSpare()
      })
    }, 0)
  }

  /** 로드를 마친 교체용 창이 있고 미리보기가 닫혀 있으면 쓴 창을 파기하고 바꾼다. 열려 있으면 다음에 닫을 때 바꾼다. */
  private swapToSpare(): void {
    const spare = this.spare
    if (!spare?.ready || this.isOpen) return
    this.spare = null
    if (this.destroyTimer) clearTimeout(this.destroyTimer)
    this.destroyTimer = null
    const old = this.win
    this.win = spare.win
    this.loaded = spare.loaded
    if (old && !old.isDestroyed()) old.destroy()
  }

  /** 쓰던 창을 파기한다(교체용 창은 남긴다). */
  private dropActive(): void {
    if (this.destroyTimer) clearTimeout(this.destroyTimer)
    this.destroyTimer = null
    this.visible = false
    this.openingSeq = 0
    this.currentPath = null
    this.native.closeAll('destroyed')
    this.loaded = null
    if (this.win && !this.win.isDestroyed()) this.win.destroy()
    this.win = null
  }

  /** 앱 종료: 모든 창을 파기하고 예약된 교체도 취소한다. */
  destroy(): void {
    if (this.recycleTimer) clearTimeout(this.recycleTimer)
    this.recycleTimer = null
    this.dropActive()
    if (this.spare && !this.spare.win.isDestroyed()) this.spare.win.destroy()
    this.spare = null
  }

  /** 개발용 스크린샷 */
  async capture(pngPath: string): Promise<void> {
    if (!this.win) return
    const image = await this.win.webContents.capturePage()
    await writeFile(pngPath, image.toPNG())
    log.info(`captured ${pngPath}`)
  }

  private ensureWindow(reason: 'show' | 'prewarm'): BrowserWindow {
    if (this.destroyTimer) clearTimeout(this.destroyTimer)
    this.destroyTimer = null
    if (this.win && !this.win.isDestroyed()) return this.win
    // 쓰던 창이 없으면(렌더러가 죽은 경우 등) 준비 중인 교체용 창을 먼저 쓴다(로드가 앞서 있다)
    const next = this.spare ?? this.createWindow(reason)
    this.spare = null
    this.win = next.win
    this.loaded = next.loaded
    return next.win
  }

  private createWindow(reason: 'show' | 'prewarm' | 'spare'): { win: BrowserWindow; loaded: Promise<boolean> } {
    const t0 = performance.now()
    const elapsed = () => `${(performance.now() - t0).toFixed(0)}ms`

    const win = new BrowserWindow({
      width: 960,
      height: 720,
      show: false,
      frame: false,
      resizable: true,
      alwaysOnTop: true,
      focusable: false,
      skipTaskbar: true,
      minimizable: false,
      maximizable: false,
      fullscreenable: false,
      // 다크 모드에서 흰 화면이 번쩍이지 않도록 렌더러 CSS 와 같은 배경
      backgroundColor: nativeTheme.shouldUseDarkColors ? '#202020' : '#f3f3f3',
      webPreferences: {
        preload: join(__dirname, '../preload/index.js'),
        sandbox: true,
        contextIsolation: true,
        nodeIntegration: false,
        spellcheck: false
      }
    })
    win.setAlwaysOnTop(true, 'floating')

    // 렌더러가 다른 곳으로 이동하거나 새 창을 여는 것을 막는다
    win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }))
    win.webContents.on('will-navigate', (e) => e.preventDefault())
    // HTML 미리보기 iframe(srcdoc)이 링크·<area>·<base> 등으로 다른 문서로 옮겨 가지 못하게 한다
    win.webContents.on('will-frame-navigate', (e) => {
      if (!e.isMainFrame && !e.url.startsWith('about:')) e.preventDefault()
    })
    win.webContents.on('render-process-gone', (_e, details) => {
      log.error(`renderer gone: ${details.reason}`)
      // 여기서는 새 창을 만들지 않는다(기동 직후 죽는 렌더러가 무한히 재생성되지 않게). 다음 show() 가 만든다.
      if (this.win === win) this.dropActive()
      else if (!win.isDestroyed()) win.destroy()
    })
    win.webContents.on('unresponsive', () => log.warn(`renderer unresponsive (pid=${win.webContents.getOSProcessId()})`))
    win.webContents.on('did-fail-load', (_e, code, desc, _url, isMainFrame) => {
      // -3(ERR_ABORTED)은 창을 닫아 로드가 취소된 경우
      if (isMainFrame && code !== -3) log.error(`renderer load failed: ${desc} (${code}) after ${elapsed()}`)
    })
    const slowTimer = setTimeout(() => {
      if (!win.isDestroyed()) log.warn(`renderer still loading after ${elapsed()} (${reason})`)
    }, SLOW_LOAD_MS)
    win.on('closed', () => {
      clearTimeout(slowTimer)
      if (this.win === win) {
        this.win = null
        this.loaded = null
        this.visible = false
      }
      if (this.spare?.win === win) this.spare = null
    })

    log.info(`window created (${reason})`)
    const devUrl = process.env['ELECTRON_RENDERER_URL']
    const loaded = (devUrl ? win.loadURL(devUrl) : win.loadFile(join(__dirname, '../renderer/index.html'))).then(
      () => {
        clearTimeout(slowTimer)
        if (win.isDestroyed()) return false
        const pid = win.webContents.getOSProcessId()
        const msg = `renderer loaded in ${elapsed()} (${reason}, pid=${pid})`
        if (performance.now() - t0 > SLOW_LOAD_MS) log.warn(msg)
        else log.info(msg)
        return true
      },
      (e) => {
        clearTimeout(slowTimer)
        // 로드 중에 창을 파기하면 로드가 ERR_FAILED 로 끝난다. 실패가 아니다.
        if (win.isDestroyed()) log.info(`renderer load cancelled after ${elapsed()} (window destroyed)`)
        else log.error('failed to load renderer', e)
        return false
      }
    )
    return { win, loaded }
  }

  /**
   * 창을 내용 비율에 맞춘다. 창이 있는 모니터 작업 영역의 85% 안에 들어가게 줄이고,
   * grow 가 아니면 원래 크기보다 키우지 않는다. 너무 작으면 최소 크기까지 늘린다(이때는 비율이 달라진다).
   */
  private fit(win: BrowserWindow, size: ContentSize, area = windowWorkArea(win)): void {
    const maxW = Math.floor(area.width * MAX_AREA_RATIO) - CHROME_W
    const maxH = Math.floor(area.height * MAX_AREA_RATIO) - CHROME_H
    let scale = Math.min(maxW / size.width, maxH / size.height)
    if (!size.grow) scale = Math.min(1, scale)
    const width = Math.max(MIN_CONTENT_W, Math.round(size.width * scale)) + CHROME_W
    const height = Math.max(MIN_CONTENT_H, Math.round(size.height * scale)) + CHROME_H
    win.setBounds({
      x: Math.round(area.x + (area.width - width) / 2),
      y: Math.round(area.y + (area.height - height) / 2),
      width,
      height
    })
  }

  /** 작업 영역(기본: 창이 있는 모니터) 중앙, 작업 영역의 70% 크기(최대 1280×900) */
  private position(win: BrowserWindow, area = windowWorkArea(win)): void {
    const width = Math.min(Math.round(area.width * 0.7), 1280)
    const height = Math.min(Math.round(area.height * 0.7), 900)
    win.setBounds({
      x: Math.round(area.x + (area.width - width) / 2),
      y: Math.round(area.y + (area.height - height) / 2),
      width,
      height
    })
  }

  private registerIpc(): void {
    const fromPreview = (sender: Electron.WebContents) => this.win !== null && sender === this.win.webContents

    ipcMain.handle(Channels.readText, async (e, path: string, maxBytes: number) => {
      if (!fromPreview(e.sender) || !this.isAllowedPath(path)) throw new Error('Forbidden')
      return readTextHead(path, maxBytes)
    })
    ipcMain.handle(Channels.listDir, async (e, path: string, max: number) => {
      if (!fromPreview(e.sender) || !this.isAllowedPath(path)) throw new Error('Forbidden')
      return listDirectory(path, max)
    })
    ipcMain.handle(Channels.listArchive, async (e, path: string, max: number) => {
      if (!fromPreview(e.sender) || !this.isAllowedPath(path)) throw new Error('Forbidden')
      return listArchive(path, max)
    })
    ipcMain.handle(Channels.nativeOpen, async (e, path: string, rect: PixelRect) => {
      if (!fromPreview(e.sender) || !this.isAllowedPath(path) || !isRect(rect)) throw new Error('Forbidden')
      // 인터넷에서 받은 파일(Mark of the Web)은 Office 등 외부 파서를 돌리지 않는다.
      // 탐색기도 같은 이유로 이런 파일의 미리 보기 창을 막는다. 샌드박스된 자체 렌더러로 보낸다.
      if (await isFromInternet(path)) return { handle: 0, status: 'unavailable', message: 'internet-zone' }
      const win = this.win
      if (!win || win.isDestroyed()) throw new Error('Forbidden')
      // 64비트 Windows 의 HWND 는 8바이트 버퍼로 온다. 실제 값은 32비트 범위라 Number 로 안전하다.
      const hwnd = Number(win.getNativeWindowHandle().readBigUInt64LE(0))
      return this.native.open(path, hwnd, rect, nativeTheme.shouldUseDarkColors)
    })
    ipcMain.on(Channels.nativeResize, (e, handle: number, rect: PixelRect) => {
      if (fromPreview(e.sender) && isRect(rect)) this.native.resize(handle, rect)
    })
    ipcMain.on(Channels.nativeClose, (e, handle: number) => {
      if (fromPreview(e.sender)) this.native.close(handle)
    })
    ipcMain.on(Channels.fit, (e, seq: number, size: unknown) => {
      const win = this.win
      if (!fromPreview(e.sender) || !win || seq !== this.seq) return
      if (size !== null && !isContentSize(size)) return
      if (!this.visible) {
        // 아직 띄우기 전: show() 가 이 크기로 띄운다
        this.firstFit = size
        this.firstFitWaiter?.()
      } else if (size === null) this.position(win)
      else this.fit(win, size)
    })
    ipcMain.on(Channels.rendered, (e, seq: number) => {
      if (!fromPreview(e.sender)) return
      const t0 = this.pendingT0.get(seq)
      if (t0 !== undefined) log.info(`rendered seq=${seq} in ${(performance.now() - t0).toFixed(0)}ms since key`)
      for (const k of this.pendingT0.keys()) if (k <= seq) this.pendingT0.delete(k)
    })
  }
}

/** 처음 띄울 때: 커서가 있는 모니터 */
function cursorWorkArea(): Electron.Rectangle {
  return screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).workArea
}

/** 이미 떠 있는 창의 크기를 바꿀 때: 창이 있는 모니터(파일을 넘길 때 다른 모니터로 옮겨 가지 않게) */
function windowWorkArea(win: BrowserWindow): Electron.Rectangle {
  return screen.getDisplayMatching(win.getBounds()).workArea
}

function isContentSize(s: unknown): s is ContentSize {
  if (typeof s !== 'object' || s === null) return false
  const { width, height, grow } = s as Record<string, unknown>
  return [width, height].every((n) => typeof n === 'number' && Number.isFinite(n) && n > 0) && (grow === undefined || typeof grow === 'boolean')
}

function isRect(r: unknown): r is PixelRect {
  if (typeof r !== 'object' || r === null) return false
  const { x, y, width, height } = r as Record<string, unknown>
  return [x, y, width, height].every((n) => typeof n === 'number' && Number.isFinite(n)) && (width as number) >= 0 && (height as number) >= 0
}

/**
 * NTFS 대체 데이터 스트림 Zone.Identifier 의 ZoneId 가 3(인터넷) 이상인지.
 * 스트림이 없거나 읽을 수 없으면(로컬 파일, FAT32 등) false.
 */
async function isFromInternet(path: string): Promise<boolean> {
  try {
    const text = await readFile(`${path}:Zone.Identifier`, 'utf8')
    const zone = /^\s*ZoneId\s*=\s*(\d+)/im.exec(text)
    return zone !== null && Number(zone[1]) >= 3
  } catch {
    return false
  }
}
