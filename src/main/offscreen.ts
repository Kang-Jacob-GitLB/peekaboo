// 화면 밖 시험 모드(PEEKABOO_OFFSCREEN=1) 좌표 계산. electron 에 의존하지 않는 순수 함수라 node --test 로 검증한다.
// 사용자가 PC 를 쓰는 중에도 화면에 아무것도 띄우지 않고 미리보기를 확인하려는 모드다.
// Tauri 쪽 같은 계산은 src-tauri/src/layout.rs 의 offscreen_origin·overlaps_any.

export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** 창을 둘 위치: 모든 모니터의 왼쪽 바깥. 어느 모니터도 그 왼쪽에는 없으므로 창이 어느 화면에도 걸치지 않는다. -32000 은 Windows 가 최소화 창에 쓰는 좌표라 피한다. */
export function offscreenOrigin(displays: Rect[], width: number): { x: number; y: number } {
  const MARGIN = 200
  const minX = displays.length ? Math.min(...displays.map((d) => d.x)) : 0
  const minY = displays.length ? Math.min(...displays.map((d) => d.y)) : 0
  return { x: minX - width - MARGIN, y: minY }
}

/** 맞닿기만 하는 경우는 겹침이 아니다 */
export function overlapsAny(win: Rect, displays: Rect[]): boolean {
  return displays.some(
    (d) => win.x < d.x + d.width && d.x < win.x + win.width && win.y < d.y + d.height && d.y < win.y + win.height
  )
}
