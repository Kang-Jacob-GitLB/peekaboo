// HTTP Range 헤더 해석(RFC 9110 14.1~14.2). electron 에 의존하지 않아 node --test 로 검사한다.

export type RangeResult =
  /** Range 가 없거나 해석할 수 없어(다중 범위 포함) 전체를 200 으로 돌려준다 */
  | { kind: 'none' }
  /** 파일 크기에 맞춰 보정한 닫힌 구간 [start, end] */
  | { kind: 'ok'; start: number; end: number }
  /** 구간이 파일 밖이다(416) */
  | { kind: 'unsatisfiable' }

/** 단일 구간(`a-b`, `a-`, `-n`)만 받는다. 형식이 이상하면 무시하고 전체를 주는 것이 규격의 허용 동작이다. */
export function parseRange(header: string | null, size: number): RangeResult {
  if (!header) return { kind: 'none' }
  const m = /^bytes=(\d*)-(\d*)$/i.exec(header.trim())
  if (!m || (m[1] === '' && m[2] === '')) return { kind: 'none' }
  const first = m[1] === '' ? null : Number(m[1])
  const second = m[2] === '' ? null : Number(m[2])
  if ((first !== null && !Number.isSafeInteger(first)) || (second !== null && !Number.isSafeInteger(second))) return { kind: 'none' }

  if (first === null) {
    // 접미 구간: 끝에서 n 바이트
    if (second === 0 || size === 0) return { kind: 'unsatisfiable' }
    return { kind: 'ok', start: Math.max(0, size - second!), end: size - 1 }
  }
  if (first >= size) return { kind: 'unsatisfiable' }
  if (second !== null && second < first) return { kind: 'none' }
  return { kind: 'ok', start: first, end: second === null ? size - 1 : Math.min(second, size - 1) }
}
