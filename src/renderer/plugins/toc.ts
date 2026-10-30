// 문서 목차 사이드바(Markdown 제목, HTML 제목, PDF 책갈피 공용).
// 본문 스크롤 영역(pane)은 container 의 직속 자식이어야 한다. 목차는 그 앞에 끼워 넣으므로 pane 을 옮기지 않는다
// (iframe 은 DOM 에서 옮기면 다시 로드된다).

/** 사이드바 폭(CSS px). 창 크기를 내용에 맞출 때 더한다. */
export const TOC_WIDTH = 240
/** 목차 항목 상한. 미리보기 파일은 신뢰하지 않으므로 제목이 수만 개여도 목록·스크롤 추적이 무거워지지 않게 자른다. */
export const TOC_MAX = 1000

export interface TocEntry {
  title: string
  /** 1 = 최상위. 가장 얕은 수준이 들여쓰기 0 이 된다. */
  level: number
  go(): void
  /** 스크롤 위치에 따라 현재 항목을 표시할 때 쓰는 제목 요소 */
  el?: Element
}

export interface TocSpy {
  /** scroll 이벤트가 나는 대상(요소 또는 iframe 의 window) */
  target: EventTarget
  /** el 의 getBoundingClientRect() 와 같은 좌표계에서 본 보이는 영역의 위쪽 끝 */
  viewTop(): number
}

// 접은 상태는 파일을 넘겨도 유지한다(미리보기 창은 숨겨질 뿐 렌더러가 살아 있다).
let collapsed = false

/** 항목이 2개 이상일 때만 목차를 붙인다. 붙였으면 해제 함수를 돌려준다. */
export function mountToc(container: HTMLElement, pane: HTMLElement, entries: TocEntry[], spy?: TocSpy): (() => void) | null {
  if (entries.length < 2) return null
  const minLevel = Math.min(...entries.map((e) => e.level))

  const nav = document.createElement('nav')
  nav.className = 'toc'
  nav.style.width = `${TOC_WIDTH}px`
  nav.setAttribute('aria-label', '목차')

  const head = document.createElement('div')
  head.className = 'toc-head'
  const label = document.createElement('span')
  label.textContent = '목차'
  const toggle = document.createElement('button')
  toggle.className = 'toc-toggle'
  head.append(label, toggle)

  const list = document.createElement('ul')
  list.className = 'toc-list'
  const items = entries.map((e) => {
    const li = document.createElement('li')
    li.textContent = e.title
    li.title = e.title
    li.style.paddingLeft = `${12 + Math.min(e.level - minLevel, 5) * 14}px`
    li.addEventListener('click', () => e.go())
    list.append(li)
    return li
  })
  nav.append(head, list)

  const apply = () => {
    nav.classList.toggle('collapsed', collapsed)
    nav.style.width = collapsed ? '' : `${TOC_WIDTH}px`
    toggle.textContent = collapsed ? '»' : '«'
    toggle.title = collapsed ? '목차 펼치기' : '목차 접기'
  }
  toggle.addEventListener('click', () => {
    collapsed = !collapsed
    apply()
  })
  apply()

  pane.classList.add('toc-pane')
  container.classList.add('has-toc')
  container.prepend(nav)

  // 현재 읽는 제목 표시: 보이는 영역 위쪽을 이미 지난 마지막 제목(아직 없으면 첫 제목)
  let active = -1
  let frame = 0
  const update = () => {
    frame = 0
    if (!spy) return
    const top = spy.viewTop() + 24
    let next = 0
    entries.forEach((e, i) => {
      if (e.el && e.el.getBoundingClientRect().top <= top) next = i
    })
    if (next === active) return
    items[active]?.classList.remove('active')
    items[next]?.classList.add('active')
    active = next
    items[next]?.scrollIntoView({ block: 'nearest' })
  }
  const onScroll = () => {
    if (!frame) frame = requestAnimationFrame(update)
  }
  spy?.target.addEventListener('scroll', onScroll, { passive: true })
  update()

  return () => {
    spy?.target.removeEventListener('scroll', onScroll)
    if (frame) cancelAnimationFrame(frame)
  }
}

/** 제목 요소(h1~h6) 목록 → 목차 항목. 내용이 비었거나 화면에 안 보이는(display:none) 제목은 뺀다. */
export function headingEntries(root: ParentNode): TocEntry[] {
  return Array.from(root.querySelectorAll('h1, h2, h3, h4, h5, h6'))
    .map((h) => ({ h, title: (h.textContent ?? '').replace(/\s+/g, ' ').trim() }))
    .filter(({ h, title }) => title && h.getClientRects().length > 0)
    .slice(0, TOC_MAX)
    .map(({ h, title }) => ({
      title,
      level: Number(h.tagName[1]),
      el: h,
      go: () => h.scrollIntoView({ block: 'start' }),
    }))
}
