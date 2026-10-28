import { app } from 'electron'
import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createLogger } from './logger'

const log = createLogger('settings')

export interface Settings {
  /** 스페이스바 단축키 사용 */
  hotkeyEnabled: boolean
  /**
   * 미리보기를 닫은 뒤 창을 숨겨 두는 시간(초). 0 이면 닫는 즉시 파기(메모리 최소).
   * 값을 주면 그 사이 다시 열 때 창 생성 비용을 건너뛴다.
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
