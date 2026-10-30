import { app } from 'electron'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createLogger } from './logger'

const log = createLogger('settings')

export interface Settings {
  /** 스페이스바 단축키 사용 */
  hotkeyEnabled: boolean
  /**
   * 미리보기를 닫은 뒤 쓴 창을 새 창으로 바꾸기까지 미루는 시간(초). 0 이면 닫는 즉시 새 창을 준비한다.
   * 어느 쪽이든 새 창은 숨긴 채 로드를 마친 뒤에야 바꾸고, 그때까지는 쓴 창을 다시 쓰므로 여는 속도와는 상관없다.
   * 미리보기용 창(렌더러 프로세스)은 늘 하나가 숨은 채 상주한다.
   */
  keepWarmSeconds: number
}

const defaults: Settings = { hotkeyEnabled: true, keepWarmSeconds: 0 }

const file = () => join(app.getPath('userData'), 'settings.json')

let current: Settings | null = null

export function getSettings(): Settings {
  if (!current) {
    try {
      current = { ...defaults, ...JSON.parse(readFileSync(file(), 'utf8')) }
    } catch {
      current = { ...defaults }
    }
  }
  return current!
}

export function updateSettings(patch: Partial<Settings>): Settings {
  current = { ...getSettings(), ...patch }
  try {
    writeFileSync(file(), JSON.stringify(current, null, 2))
  } catch (e) {
    log.error('failed to save settings', e)
  }
  return current
}

// 시작 프로그램 등록은 OS 상태(레지스트리 Run 키)가 단일 출처 — 설정 파일에 따로 저장하지 않는다.
export function getLaunchAtStartup(): boolean {
  return app.getLoginItemSettings().openAtLogin
}

export function setLaunchAtStartup(enabled: boolean): void {
  app.setLoginItemSettings({ openAtLogin: enabled })
}
