import { app, Menu, Tray, nativeImage, shell } from 'electron'
import { join } from 'node:path'
import type { HelperStatus } from './helper-supervisor'
import { logDir } from './logger'
import { getLaunchAtStartup, getSettings, setLaunchAtStartup, updateSettings } from './settings'

const statusText: Record<HelperStatus, string> = {
  starting: '시작 중…',
  running: '동작 중',
  restarting: '헬퍼 재시작 중…',
  missing: '헬퍼 실행 파일 없음',
  stopped: '중지됨'
}

function iconPath(): string {
  return app.isPackaged ? join(process.resourcesPath, 'tray.png') : join(app.getAppPath(), 'resources', 'tray.png')
}

export class AppTray {
  private tray: Tray
  private status: HelperStatus = 'starting'

  constructor(private readonly onHotkeyToggle: (enabled: boolean) => void) {
    this.tray = new Tray(nativeImage.createFromPath(iconPath()))
    this.refresh()
  }

  setStatus(status: HelperStatus): void {
    this.status = status
    this.refresh()
  }

  private refresh(): void {
    const { hotkeyEnabled } = getSettings()
    const state = hotkeyEnabled ? statusText[this.status] : '단축키 꺼짐'
    this.tray.setToolTip(`Peekaboo — ${state}`)
    this.tray.setContextMenu(
      Menu.buildFromTemplate([
        { label: `Peekaboo ${app.getVersion()} — ${state}`, enabled: false },
        { type: 'separator' },
        {
          label: '스페이스바로 미리보기',
          type: 'checkbox',
          checked: hotkeyEnabled,
          click: (item) => {
            updateSettings({ hotkeyEnabled: item.checked })
            this.onHotkeyToggle(item.checked)
            this.refresh()
          }
        },
        {
          label: 'Windows 시작 시 실행',
          type: 'checkbox',
          checked: getLaunchAtStartup(),
          // 개발 모드에서는 electron.exe 가 등록되므로 막는다
          enabled: app.isPackaged,
          click: (item) => {
            setLaunchAtStartup(item.checked)
            this.refresh()
          }
        },
        { type: 'separator' },
        { label: '로그 폴더 열기', click: () => void shell.openPath(logDir()) },
        { label: '종료', click: () => app.quit() }
      ])
    )
  }
}
