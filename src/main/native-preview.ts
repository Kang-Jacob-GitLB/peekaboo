import type { HostMessage, PixelRect } from '../shared/ipc-types'
import type { NativePreviewResult } from '../shared/preview-api'
import { createLogger } from './logger'

const log = createLogger('native')

/** 헬퍼가 응답하지 않을 때 렌더러가 대체 렌더러로 넘어가도록 하는 상한 */
const OPEN_TIMEOUT_MS = 8000

/**
 * Windows 미리보기 핸들러 요청을 헬퍼로 중계한다.
 * 핸들러는 헬퍼(전용 STA 스레드)가 띄우고, 핸들러 창은 미리보기 창 HWND 의 자식으로 붙는다.
 * 동시에 하나만 연다 — 새로 열면 헬퍼가 이전 것을 Unload 한다.
 */
export class NativePreviewBridge {
  private nextId = 1
  private current: number | null = null
  private pending = new Map<number, { resolve: (r: NativePreviewResult) => void; timer: NodeJS.Timeout }>()

  constructor(private readonly send: (msg: HostMessage) => boolean) {}

  open(path: string, hwnd: number, rect: PixelRect, dark: boolean): Promise<NativePreviewResult> {
    const id = this.nextId++
    this.current = id
    // 헬퍼가 죽었거나 재시작 중이면 타임아웃까지 기다리지 않고 바로 자체 렌더러로 넘긴다
    if (!this.send({ v: 1, type: 'nativePreview.open', id, path, hwnd, rect, dark })) {
      this.current = null
      return Promise.resolve({ handle: id, status: 'failed', message: 'helper not running' })
    }
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        log.warn(`open #${id} timed out`)
        this.close(id)
        resolve({ handle: id, status: 'failed', message: 'timeout' })
      }, OPEN_TIMEOUT_MS)
      this.pending.set(id, { resolve, timer })
    })
  }

  resize(id: number, rect: PixelRect): void {
    if (id === this.current) this.send({ v: 1, type: 'nativePreview.resize', id, rect })
  }

  close(id: number): void {
    if (id !== this.current) return
    this.current = null
    this.send({ v: 1, type: 'nativePreview.close', id })
  }

  /** 미리보기 창이 닫히거나 헬퍼가 재시작됐을 때. 대기 중인 요청은 failed 로 끝낸다. */
  closeAll(reason: string): void {
    if (this.current !== null) this.close(this.current)
    for (const [id, p] of this.pending) {
      clearTimeout(p.timer)
      p.resolve({ handle: id, status: 'failed', message: reason })
    }
    this.pending.clear()
  }

  /** 헬퍼의 nativePreview 응답 */
  onResult(id: number, status: NativePreviewResult['status'], message?: string): void {
    const p = this.pending.get(id)
    if (!p) return
    clearTimeout(p.timer)
    this.pending.delete(id)
    if (status !== 'shown') log.info(`#${id} ${status}${message ? `: ${message}` : ''}`)
    p.resolve({ handle: id, status, message })
  }
}
