import { app } from 'electron'
import { spawn, type ChildProcess } from 'node:child_process'
import { EventEmitter } from 'node:events'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import type { HelperMessage, HostMessage } from '../shared/ipc-types'
import { createLogger } from './logger'

const log = createLogger('helper')
const helperLog = createLogger('helper.exe')

const PING_INTERVAL_MS = 2000
const MAX_MISSED_PONGS = 3
const BACKOFF_MS = [1000, 2000, 4000, 8000, 15000, 30000]
const STABLE_RESET_MS = 60_000 // 이만큼 살아 있으면 백오프 단계를 초기화

/** 패키지: resources/helper, 개발: helper/publish → helper/bin/Debug 순으로 찾는다. */
function resolveHelperPath(): string | null {
  const candidates = app.isPackaged
    ? [join(process.resourcesPath, 'helper', 'peekaboo-helper.exe')]
    : [
        join(app.getAppPath(), 'helper', 'publish', 'peekaboo-helper.exe'),
        join(app.getAppPath(), 'helper', 'bin', 'Debug', 'net8.0-windows', 'win-x64', 'peekaboo-helper.exe')
      ]
  return candidates.find((p) => existsSync(p)) ?? null
}

export type HelperStatus = 'starting' | 'running' | 'restarting' | 'missing' | 'stopped'

type HelperEvents = {
  message: [HelperMessage]
  status: [HelperStatus]
}

/**
 * 헬퍼 프로세스 수명 관리: 실행, stdout JSON Lines 파싱, ping 감시, 비정상 종료 시 백오프 재시작.
 * 다시 실행되면 'ready' 메시지가 오므로, 소비자는 그때 설정/상태를 재전송하면 된다.
 */
export class HelperSupervisor extends EventEmitter<HelperEvents> {
  private child: ChildProcess | null = null
  private pingTimer: NodeJS.Timeout | null = null
  private restartTimer: NodeJS.Timeout | null = null
  private nextPingId = 1
  private missedPongs = 0
  private backoffStep = 0
  private startedAt = 0
  private stopping = false
  status: HelperStatus = 'stopped'

  start(): void {
    this.stopping = false
    this.launch()
  }

  /** 헬퍼가 떠 있지 않으면 보내지 않고 false */
  send(msg: HostMessage): boolean {
    const stdin = this.child?.stdin
    if (!stdin || stdin.destroyed) return false
    stdin.write(JSON.stringify(msg) + '\n')
    return true
  }

  stop(): void {
    this.stopping = true
    if (this.restartTimer) clearTimeout(this.restartTimer)
    this.stopPing()
    const child = this.child
    if (!child) return
    this.send({ v: 1, type: 'shutdown' })
    child.stdin?.end()
    // 정상 종료를 잠깐 기다리고 안 되면 강제 종료
    setTimeout(() => {
      if (child.exitCode === null) child.kill()
    }, 1000).unref()
    this.setStatus('stopped')
  }

  private setStatus(status: HelperStatus): void {
    if (this.status === status) return
    this.status = status
    this.emit('status', status)
  }

  private launch(): void {
    const exe = resolveHelperPath()
    if (!exe) {
      // 경로가 없으면 재시작해도 소용없으니 루프를 돌지 않는다
      log.error('helper executable not found (run `npm run helper:build`)')
      this.setStatus('missing')
      return
    }

    this.setStatus(this.backoffStep === 0 ? 'starting' : 'restarting')
    log.info(`spawning ${exe}`)
    const child = spawn(exe, [], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true })
    this.child = child
    this.startedAt = Date.now()
    this.missedPongs = 0

    createInterface({ input: child.stdout! }).on('line', (line) => this.onLine(line))
    createInterface({ input: child.stderr! }).on('line', (line) => helperLog.raw(line))

    child.on('error', (err) => log.error('helper process error', err))
    child.on('exit', (code, signal) => this.onExit(child, code, signal))
    child.stdin?.on('error', () => {
      // 헬퍼가 먼저 죽으면 EPIPE — exit 핸들러가 처리
    })
  }

  private onLine(line: string): void {
    if (!line.trim()) return
    let msg: HelperMessage
    try {
      msg = JSON.parse(line)
    } catch {
      log.warn(`unparseable helper output: ${line.slice(0, 200)}`)
      return
    }
    switch (msg.type) {
      case 'ready':
        log.info(`helper ready v${msg.version} pid=${msg.pid}`)
        this.setStatus('running')
        this.startPing()
        break
      case 'pong':
        this.missedPongs = 0
        return
      case 'error':
        log.warn(`helper error ${msg.code}: ${msg.message}`)
        break
    }
    this.emit('message', msg)
  }

  private onExit(child: ChildProcess, code: number | null, signal: NodeJS.Signals | null): void {
    if (this.child !== child) return
    this.child = null
    this.stopPing()
    if (this.stopping) return

    if (Date.now() - this.startedAt > STABLE_RESET_MS) this.backoffStep = 0
    const delay = BACKOFF_MS[Math.min(this.backoffStep, BACKOFF_MS.length - 1)]
    this.backoffStep++
    log.warn(`helper exited (code=${code} signal=${signal}); restarting in ${delay}ms`)
    this.setStatus('restarting')
    this.restartTimer = setTimeout(() => this.launch(), delay)
  }

  private startPing(): void {
    this.stopPing()
    this.pingTimer = setInterval(() => {
      if (this.missedPongs >= MAX_MISSED_PONGS) {
        log.warn('helper unresponsive; killing')
        this.child?.kill() // exit 핸들러가 재시작
        return
      }
      this.missedPongs++
      this.send({ v: 1, type: 'ping', id: this.nextPingId++ })
    }, PING_INTERVAL_MS)
  }

  private stopPing(): void {
    if (this.pingTimer) clearInterval(this.pingTimer)
    this.pingTimer = null
  }
}
