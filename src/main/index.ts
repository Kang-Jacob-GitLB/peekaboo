import { app, Menu, Tray, nativeImage } from 'electron'
import { join } from 'node:path'

let tray: Tray | null = null

function trayIconPath(): string {
  return app.isPackaged ? join(process.resourcesPath, 'tray.png') : join(app.getAppPath(), 'resources', 'tray.png')
}

if (!app.requestSingleInstanceLock()) {
  app.quit()
} else {
  app.whenReady().then(() => {
    tray = new Tray(nativeImage.createFromPath(trayIconPath()))
    tray.setToolTip('Peekaboo')
    tray.setContextMenu(Menu.buildFromTemplate([{ label: '종료', click: () => app.quit() }]))
  })

  // 창이 모두 닫혀도 트레이에 상주
  app.on('window-all-closed', () => {})
}
