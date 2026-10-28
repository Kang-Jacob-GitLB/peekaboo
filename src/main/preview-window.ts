import { BrowserWindow, ipcMain, nativeTheme, screen } from 'electron'
import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { Channels } from '../shared/channels'
import type { ShowPayload } from '../shared/preview-api'
import { getFileInfo, listDirectory, readTextHead } from './file-service'
import { createLogger } from './logger'
import type { NativePreviewBridge } from './native-preview'
import type { PixelRect } from '../shared/ipc-types'

const log = createLogger('preview')

export interface PreviewRequest {
  path: string
  selectionCount: number
  /** 지연 측정 시작점(키 입력 수신 시각, performance.now()) */
  t0: number
}

/**
 * 미리보기 창 하나를 관리한다.
 * - 프레임 없음, 항상 위, 포커스를 가져가지 않음(focusable:false + showInactive) → 탐색기 포커스 유지
 * - 필요할 때 생성하고 닫으면 파기. keepWarmSeconds > 0 이면 그 시간 동안만 숨겨 둔다.
 */
export class PreviewWindow {
  private win: BrowserWindow | null = null
  private loaded: Promise<void> | null = null
  private destroyTimer: NodeJS.Timeout | null = null
  private seq = 0
  private currentPath: string | null = null
  private pendingT0 = new Map<number, number>()
  private visible = false

  constructor(
    private readonly keepWarmSeconds: () => number,
    private readonly native: NativePreviewBridge
  ) {
    this.registerIpc()
  }

  get isOpen(): boolean {
    return this.visible
  }

  /** peek:// 와 readText 가 허용하는 유일한 경로 */
  isAllowedPath(path: string): boolean {
    return this.currentPath !== null && path.toLowerCase() === this.currentPath.toLowerCase()
  }

  async show(req: PreviewRequest): Promise<void> {
    const seq = ++this.seq
    this.pendingT0.set(seq, req.t0)
    this.currentPath = req.path

    const win = this.ensureWindow()
    let payload: ShowPayload | null = null
    let error: string | null = null
    try {
      payload = { seq, file: await getFileInfo(req.path), selectionCount: req.selectionCount }
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
    await this.loaded
    if (seq !== this.seq || win.isDestroyed()) return // 그 사이 다른 파일이 요청됨

    if (payload) win.webContents.send(Channels.show, payload)
    else win.webContents.send(Channels.showError, { seq, path: req.path, message: error })

    if (!this.visible) {
      this.position(win)
      win.showInactive()
      this.visible = true
      log.info(`shown in ${(performance.now() - req.t0).toFixed(0)}ms since key`)
    }
  }

  hide(): void {
    if (!this.win || !this.visible) return
    this.visible = false
    this.currentPath = null
    this.native.closeAll('hidden')
    this.win.webContents.send(Channels.hide)
    const keep = this.keepWarmSeconds()
    if (keep <= 0) {
      this.destroy()
      return
    }
    this.win.hide()
    this.destroyTimer = setTimeout(() => this.destroy(), keep * 1000)
  }

  destroy(): void {
    if (this.destroyTimer) clearTimeout(this.destroyTimer)
    this.destroyTimer = null
    this.visible = false
    this.currentPath = null
    this.native.closeAll('destroyed')
    this.loaded = null
    if (this.win && !this.win.isDestroyed()) this.win.destroy()
    this.win = null
  }

  /** 개발용 스크린샷 */
  async capture(pngPath: string): Promise<void> {
    if (!this.win) return
    const image = await this.win.webContents.capturePage()
    await writeFile(pngPath, image.toPNG())
    log.info(`captured ${pngPath}`)
  }

  private ensureWindow(): BrowserWindow {
    if (this.destroyTimer) clearTimeout(this.destroyTimer)
    this.destroyTimer = null
    if (this.win && !this.win.isDestroyed()) return this.win

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
    win.webContents.on('render-process-gone', (_e, details) => {
      log.error(`renderer gone: ${details.reason}`)
      this.destroy()
    })
    win.on('closed', () => {
      if (this.win === win) {
        this.win = null
        this.visible = false
      }
    })

    const devUrl = process.env['ELECTRON_RENDERER_URL']
    this.loaded = (devUrl ? win.loadURL(devUrl) : win.loadFile(join(__dirname, '../renderer/index.html'))).catch(
      (e) => log.error('failed to load renderer', e)
    )
    this.win = win
    return win
  }

  /** 커서가 있는 모니터의 작업 영역 중앙, 작업 영역의 70% 크기(최대 1280×900) */
  private position(win: BrowserWindow): void {
    const area = screen.getDisplayNearestPoint(screen.getCursorScreenPoint()).workArea
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
    ipcMain.on(Channels.rendered, (e, seq: number) => {
      if (!fromPreview(e.sender)) return
      const t0 = this.pendingT0.get(seq)
      if (t0 !== undefined) log.info(`rendered seq=${seq} in ${(performance.now() - t0).toFixed(0)}ms since key`)
      for (const k of this.pendingT0.keys()) if (k <= seq) this.pendingT0.delete(k)
    })
  }
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
