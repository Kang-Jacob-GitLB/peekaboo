// 화면 밖 시험 모드(PEEKABOO_OFFSCREEN=1)의 좌표: 어느 모니터와도 겹치지 않아야 한다.

import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { offscreenOrigin, overlapsAny } from '../src/main/offscreen.ts'

describe('offscreen', () => {
  const displays = [
    { x: 0, y: 0, width: 1920, height: 1080 },
    { x: -1280, y: -200, width: 1280, height: 1024 },
    { x: 1920, y: 100, width: 2560, height: 1440 }
  ]

  it('화면 밖 좌표는 어느 모니터와도 겹치지 않는다', () => {
    for (const width of [420, 1200, 9000]) {
      const { x, y } = offscreenOrigin(displays, width)
      assert.equal(overlapsAny({ x, y, width, height: 800 }, displays), false, `width=${width}`)
    }
    assert.deepEqual(offscreenOrigin([], 100), { x: -300, y: 0 })
  })

  it('겹침 검사는 걸치는 창을 잡고 맞닿기만 하면 통과시킨다', () => {
    const m = [{ x: 0, y: 0, width: 100, height: 100 }]
    assert.equal(overlapsAny({ x: 99, y: 99, width: 10, height: 10 }, m), true)
    assert.equal(overlapsAny({ x: -5, y: -5, width: 10, height: 10 }, m), true)
    assert.equal(overlapsAny({ x: 10, y: 10, width: 5, height: 5 }, m), true)
    assert.equal(overlapsAny({ x: 100, y: 0, width: 10, height: 10 }, m), false)
    assert.equal(overlapsAny({ x: -10, y: 0, width: 10, height: 10 }, m), false)
    assert.equal(overlapsAny({ x: 0, y: 0, width: 10, height: 10 }, []), false)
  })
})
