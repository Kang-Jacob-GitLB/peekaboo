import { app } from 'electron'
import { appendFileSync, mkdirSync, renameSync, statSync } from 'node:fs'
import { join } from 'node:path'

// %APPDATA%\Peekaboo\logs\peekaboo.log — 2MB 넘으면 .1 로 한 번 회전
const MAX_BYTES = 2 * 1024 * 1024

let file: string | null = null

export function logDir(): string {
  return join(app.getPath('userData'), 'logs')
}

function target(): string {
  if (!file) {
    mkdirSync(logDir(), { recursive: true })
    file = join(logDir(), 'peekaboo.log')
  }
  return file
}

function rotateIfNeeded(path: string): void {
  try {
    if (statSync(path).size > MAX_BYTES) renameSync(path, `${path}.1`)
  } catch {
    // 파일이 아직 없음
  }
}

function write(level: string, scope: string, msg: string): void {
  const line = `${new Date().toISOString()} [${level}] [${scope}] ${msg}\n`
  if (!app.isPackaged) process.stdout.write(line)
  try {
    const path = target()
    rotateIfNeeded(path)
    appendFileSync(path, line)
  } catch {
    // 로그 실패로 앱을 멈추지 않는다
  }
}

export function createLogger(scope: string) {
  return {
    info: (msg: string) => write('INFO', scope, msg),
    warn: (msg: string) => write('WARN', scope, msg),
    error: (msg: string, err?: unknown) =>
      write('ERROR', scope, err ? `${msg}: ${err instanceof Error ? (err.stack ?? err.message) : String(err)}` : msg),
    /** 헬퍼 stderr 처럼 이미 레벨/시간이 붙은 줄을 그대로 남길 때 */
    raw: (msg: string) => write('RAW', scope, msg)
  }
}

export type Logger = ReturnType<typeof createLogger>
