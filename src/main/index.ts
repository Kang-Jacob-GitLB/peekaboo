import { app } from 'electron'
import type { HelperMessage } from '../shared/ipc-types'
import { HelperSupervisor } from './helper-supervisor'
import { NativePreviewBridge } from './native-preview'
import { createLogger } from './logger'
import { PreviewWindow } from './preview-window'
import { blockRemoteFileRequests, handlePeekProtocol, registerPeekScheme } from './protocol'
import { getSettings } from './settings'
import { AppTray } from './tray'
import { Updater } from './updater'

const log = createLogger('main')

process.on('uncaughtException', (err) => log.error('uncaught exception', err))
process.on('unhandledRejection', (err) => log.error('unhandled rejection', err))

registerPeekScheme()

if (!app.requestSingleInstanceLock()) {
  app.quit()
} else {
  app.whenReady().then(start)
  // 창이 모두 닫혀도 트레이에 상주
  app.on('window-all-closed', () => {})
}

function start(): void {
  log.info(`Peekaboo ${app.getVersion()} starting (packaged=${app.isPackaged})`)
  // 알림이 "Peekaboo" 로 뜨도록 설치 프로그램이 만든 바로 가기와 같은 AppUserModelID 를 쓴다(electron-builder 의 appId)
  app.setAppUserModelId('io.github.kang-jacob-gitlb.peekaboo')

  const helper = new HelperSupervisor()
  const native = new NativePreviewBridge((msg) => helper.send(msg))
  const preview = new PreviewWindow(() => getSettings().keepWarmSeconds, native)
  handlePeekProtocol((path) => preview.isAllowedPath(path))
  blockRemoteFileRequests((path) => preview.isAllowedPath(path))

  const updater = new Updater(() => tray.refreshUpdate())
  const tray = new AppTray((enabled) => {
    helper.send({ v: 1, type: 'config', enabled })
    if (!enabled) closePreview()
  }, updater)
  helper.on('status', (s) => tray.setStatus(s))
  updater.start()

  let nextRequestId = 1
  /** getSelection 요청 id → 키 수신 시각 */
  const pendingOpen = new Map<number, number>()

  function openPreview(): void {
    const id = nextRequestId++
    pendingOpen.set(id, performance.now())
    helper.send({ v: 1, type: 'getSelection', id })
  }

  function closePreview(): void {
    pendingOpen.clear()
    if (!preview.isOpen) return
    preview.hide()
    helper.send({ v: 1, type: 'state', previewOpen: false })
  }

  function showSelection(paths: string[], focused: string | null, t0: number): void {
    const path = focused && paths.includes(focused) ? focused : (paths[0] ?? focused)
    if (!path) return // 선택 없음
    // Esc 삼키기와 선택 변경 감시가 이 상태에 의존하므로 창을 띄우기 전에 알린다
    if (!preview.isOpen) helper.send({ v: 1, type: 'state', previewOpen: true })
    void preview.show({ path, selectionCount: paths.length, t0 })
  }

  helper.on('message', (msg: HelperMessage) => {
    switch (msg.type) {
      case 'ready':
        // 헬퍼가 (재)시작됨 → 이전 헬퍼가 띄운 미리보기 핸들러는 사라졌으므로 대기 중인 요청을 끝낸다
        native.closeAll('helper restarted')
        // 현재 설정/상태를 다시 알린다
        helper.send({ v: 1, type: 'config', enabled: getSettings().hotkeyEnabled })
        helper.send({ v: 1, type: 'state', previewOpen: preview.isOpen })
        break
      case 'key':
        // 여는 중(getSelection 응답 대기)에 다시 Space 를 누르면 취소로 본다
        if (msg.key === 'space') preview.isOpen || pendingOpen.size > 0 ? closePreview() : openPreview()
        else if (msg.key === 'esc' || msg.key === 'enter') closePreview()
        // 방향키: 탐색기가 선택을 옮기고, 헬퍼가 selection 알림을 보낸다
        break
      case 'selection':
        if (msg.id !== undefined) {
          const t0 = pendingOpen.get(msg.id)
          if (t0 === undefined) return // 이미 닫힘/취소됨
          pendingOpen.delete(msg.id)
          showSelection(msg.paths, msg.focused, t0)
        } else if (preview.isOpen) {
          showSelection(msg.paths, msg.focused, performance.now())
        }
        break
      case 'nativePreview':
        native.onResult(msg.id, msg.status, msg.message)
        break
      case 'foreground':
        // 다른 앱으로 전환하면 닫는다(띄우는 중이면 취소). 이 앱 자신의 창(트레이 메뉴 등)은 제외.
        // 미리보기 창은 focusable:false 라 클릭해도 포그라운드가 되지 않는다.
        if (msg.pid !== process.pid && (preview.isOpen || pendingOpen.size > 0)) {
          log.info(`foreground moved to pid ${msg.pid} → close`)
          closePreview()
        }
        break
      case 'error':
        if (msg.id !== undefined) pendingOpen.delete(msg.id)
        break
    }
  })

  helper.start()

  // 개발 전용: 헬퍼/키 입력 없이 미리보기 창만 확인한다.
  //   PEEKABOO_PREVIEW=<파일 경로> [PEEKABOO_CAPTURE=<png 경로>] npx electron .
  const devPreview = process.env['PEEKABOO_PREVIEW']
  if (!app.isPackaged && devPreview) {
    showSelection([devPreview], devPreview, performance.now())
    const capture = process.env['PEEKABOO_CAPTURE']
    if (capture) setTimeout(() => void preview.capture(capture), 2500)
  }

  app.on('before-quit', () => {
    helper.stop()
    preview.destroy()
  })
}
