import { app, Notification } from 'electron'
import { autoUpdater } from 'electron-updater'
import { createLogger } from './logger'

const log = createLogger('update')

/** 시작 직후 첫 확인까지(부팅 직후 네트워크·디스크 경합을 피한다) */
const FIRST_CHECK_MS = 10_000
/** 이후 주기적 확인 — 트레이에 오래 상주하므로 */
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000

export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'latest' }
  | { kind: 'downloading'; version: string; percent: number }
  | { kind: 'ready'; version: string }
  | { kind: 'error' }

/**
 * GitHub Releases(electron-builder.yml 의 publish)에서 새 버전을 찾아 자동으로 내려받는다.
 * 설치는 트레이 메뉴·알림의 "다시 시작하여 업데이트"로 바로 하거나, 그냥 두면 앱을 끌 때 조용히 설치된다.
 * 설치 프로그램 무결성은 latest.yml 의 sha512 로 검증된다(코드 서명 없음).
 */
export class Updater {
  private state: UpdateState = { kind: 'idle' }
  /** 사람이 트레이에서 누른 확인이면 결과(최신·오류)를 알림으로 알려 준다. 자동 확인은 조용히 넘어간다. */
  private manual = false
  private notifiedVersion: string | null = null
  private notice: Notification | null = null

  constructor(private readonly onChange: (state: UpdateState) => void) {}

  get current(): UpdateState {
    return this.state
  }

  /** 설치본에서만 동작한다(개발 실행에는 app-update.yml 이 없다). */
  get enabled(): boolean {
    return app.isPackaged
  }

  start(): void {
    if (!this.enabled) return
    autoUpdater.autoDownload = true
    autoUpdater.autoInstallOnAppQuit = true
    autoUpdater.allowDowngrade = false
    // 프리릴리스(vX.Y.Z-rc.N)는 지금 프리릴리스를 쓰는 경우에만 받는다
    autoUpdater.allowPrerelease = app.getVersion().includes('-')
    autoUpdater.logger = {
      info: (m: unknown) => log.info(String(m)),
      warn: (m: unknown) => log.warn(String(m)),
      error: (m: unknown) => log.error(String(m)),
      debug: () => {}
    }

    autoUpdater.on('checking-for-update', () => this.set({ kind: 'checking' }))
    autoUpdater.on('update-not-available', () => {
      this.set({ kind: 'latest' })
      if (this.manual) this.notify('Peekaboo 가 최신 버전입니다', `현재 버전 ${app.getVersion()}`)
      this.manual = false
    })
    autoUpdater.on('update-available', (info) => {
      this.set({ kind: 'downloading', version: info.version, percent: 0 })
      if (this.manual) this.notify(`새 버전 ${info.version} 을 내려받는 중입니다`, '끝나면 다시 알려 드립니다.')
      this.manual = false
    })
    autoUpdater.on('download-progress', (p) => {
      if (this.state.kind !== 'downloading') return
      // 트레이 메뉴를 너무 자주 다시 만들지 않도록 10% 단위로만 갱신
      const percent = Math.floor(p.percent / 10) * 10
      if (percent !== this.state.percent) this.set({ ...this.state, percent })
    })
    autoUpdater.on('update-downloaded', (info) => {
      this.set({ kind: 'ready', version: info.version })
      if (this.notifiedVersion === info.version) return
      this.notifiedVersion = info.version
      this.notify(
        `Peekaboo ${info.version} 업데이트 준비됨`,
        '클릭하면 다시 시작해 설치합니다. 그냥 두면 Peekaboo 를 끌 때 설치됩니다.',
        () => this.install()
      )
    })
    // 리스너가 없으면 EventEmitter 가 error 를 던진다
    autoUpdater.on('error', (err) => {
      log.warn(`update failed: ${err?.message ?? err}`)
      // 이미 내려받은 업데이트는 다음 확인이 실패해도 유효하다
      if (this.state.kind !== 'ready') this.set({ kind: 'error' })
      if (this.manual) this.notify('업데이트를 확인하지 못했습니다', '인터넷 연결을 확인한 뒤 다시 시도해 주세요. 자세한 내용은 로그 폴더에 있습니다.')
      this.manual = false
    })

    setTimeout(() => this.check(false), FIRST_CHECK_MS).unref()
    setInterval(() => this.check(false), CHECK_INTERVAL_MS).unref()
  }

  check(manual: boolean): void {
    if (!this.enabled) return
    const busy = this.state.kind === 'checking' || this.state.kind === 'downloading'
    if (busy || this.state.kind === 'ready') {
      // 이미 진행 중이거나 받아 둔 것이 있으면 다시 확인하지 않는다(받아 둔 버전은 메뉴에서 바로 설치)
      if (manual && this.state.kind === 'ready') this.install()
      return
    }
    this.manual = manual
    autoUpdater.checkForUpdates().catch(() => {
      // 'error' 이벤트에서 처리한다
    })
  }

  install(): void {
    if (this.state.kind !== 'ready') return
    log.info(`installing ${this.state.version}`)
    // isSilent: 설치 화면 없이, isForceRunAfter: 설치 뒤 다시 실행. 앱 종료(before-quit)에서 헬퍼도 내려간다.
    autoUpdater.quitAndInstall(true, true)
  }

  private set(state: UpdateState): void {
    this.state = state
    this.onChange(state)
  }

  private notify(title: string, body: string, onClick?: () => void): void {
    if (!Notification.isSupported()) return
    const n = new Notification({ title, body })
    if (onClick) n.on('click', onClick)
    // 참조가 없으면 GC 되어 클릭 처리기가 사라질 수 있다
    this.notice = n
    n.show()
  }
}
