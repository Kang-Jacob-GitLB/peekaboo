#!/usr/bin/env node
// 다음 릴리스 버전과 릴리스 노트를 계산한다(야간 자동 릴리스 .github/workflows/nightly-release.yml 이 쓴다).
// 기준: main 에서 닿는 마지막 정식 태그(vX.Y.Z, 프리릴리스 제외) 이후의 커밋 제목(= squash 머지된 PR 제목, Conventional Commits).
//   feat                 → minor
//   fix · perf · style   → patch      (style = 미리보기 화면·디자인 변경)
//   type! · BREAKING CHANGE → major
//   그 밖(ci·docs·chore·test·refactor·build)만 있으면 릴리스하지 않는다 — 문서·빌드 설정만 바뀐 날은 건너뛴다
// 사용: node scripts/next-version.mjs [--bump auto|patch|minor|major] [--since vX.Y.Z] [--json]
//   --json 이면 {release, bump, last, next, notes} 를 출력한다. 아니면 사람이 읽는 요약.
import { execFileSync } from 'node:child_process'

const SEMVER = /^v(\d+)\.(\d+)\.(\d+)$/
const CC = /^(\w+)(?:\(([^)]*)\))?(!)?:\s*(.+)$/

/** 커밋 하나의 종류와 올림 단계 */
export function classify(subject, body = '') {
  const m = CC.exec(subject.trim())
  const type = m ? m[1].toLowerCase() : 'other'
  const breaking = !!(m && m[3]) || /(^|\n)BREAKING[ -]CHANGE:/.test(body)
  let bump = null
  if (breaking) bump = 'major'
  else if (type === 'feat') bump = 'minor'
  else if (type === 'fix' || type === 'perf' || type === 'style') bump = 'patch'
  return { type, scope: m?.[2] ?? null, breaking, bump, title: m ? m[4] : subject.trim() }
}

const RANK = { patch: 1, minor: 2, major: 3 }

/** 여러 커밋 중 가장 큰 올림 단계(없으면 null) */
export function decideBump(commits) {
  let best = null
  for (const c of commits) {
    const b = classify(c.subject, c.body).bump
    if (b && (!best || RANK[b] > RANK[best])) best = b
  }
  return best
}

/** 'vX.Y.Z' 에 올림을 적용한 다음 버전(접두사 없이) */
export function bumpVersion(lastTag, bump) {
  const m = SEMVER.exec(lastTag)
  if (!m) throw new Error(`정식 semver 태그가 아닙니다: ${lastTag}`)
  let [maj, min, pat] = [Number(m[1]), Number(m[2]), Number(m[3])]
  if (bump === 'major') [maj, min, pat] = [maj + 1, 0, 0]
  else if (bump === 'minor') [min, pat] = [min + 1, 0]
  else if (bump === 'patch') pat += 1
  else throw new Error(`알 수 없는 올림: ${bump}`)
  return `${maj}.${min}.${pat}`
}

const SECTIONS = [
  ['breaking', '호환성이 바뀌는 변경'],
  ['feat', '새 기능'],
  ['fix', '버그 수정'],
  ['perf', '성능'],
  ['style', '화면·디자인'],
  ['other', '그 밖의 변경(내부)']
]

/** 릴리스 노트(마크다운). 첫 줄은 태그 제목 */
export function releaseNotes(next, commits) {
  const groups = Object.fromEntries(SECTIONS.map(([k]) => [k, []]))
  for (const c of commits) {
    const k = classify(c.subject, c.body)
    const key = k.breaking ? 'breaking' : k.type in groups ? k.type : 'other'
    groups[key].push(`- ${k.scope ? `**${k.scope}**: ` : ''}${k.title}`)
  }
  const out = [`v${next}`, '']
  for (const [k, label] of SECTIONS) {
    if (!groups[k].length) continue
    out.push(`## ${label}`, '', ...groups[k], '')
  }
  out.push('설치 파일은 아래 Assets 에 있습니다. 0.2.0 이상을 쓰고 있다면 앱이 이 버전을 자동으로 내려받고, 트레이 메뉴의 "다시 시작하여 업데이트"로 설치하거나 Peekaboo 를 끌 때 설치됩니다.')
  return out.join('\n')
}

function git(...args) {
  return execFileSync('git', args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).trim()
}

/** HEAD 에서 닿는 마지막 정식 태그(vX.Y.Z). 프리릴리스(-rc 등)는 건너뛴다 */
export function lastReleaseTag() {
  const tags = git('tag', '--merged', 'HEAD', '--list', 'v*', '--sort=-v:refname').split('\n')
  return tags.find((t) => SEMVER.test(t)) ?? null
}

/** 태그 이후 커밋(오래된 것부터). 머지 커밋은 뺀다(main 은 squash 만 쓴다) */
export function commitsSince(tag) {
  const raw = git('log', '--no-merges', '--reverse', '--format=%H%x1f%s%x1f%b%x1e', `${tag}..HEAD`)
  return raw
    .split('\x1e')
    .map((r) => r.trim())
    .filter(Boolean)
    .map((r) => {
      const [hash, subject, body] = r.split('\x1f')
      return { hash, subject: subject ?? '', body: body ?? '' }
    })
}

function main() {
  const args = process.argv.slice(2)
  const json = args.includes('--json')
  const i = args.indexOf('--bump')
  const forced = i >= 0 ? args[i + 1] : 'auto'
  if (!['auto', 'patch', 'minor', 'major'].includes(forced)) throw new Error(`--bump 값이 잘못됐습니다: ${forced}`)

  // --since vX.Y.Z: 기준 태그를 직접 지정(점검용)
  const s = args.indexOf('--since')
  const last = s >= 0 ? args[s + 1] : lastReleaseTag()
  if (!last || !SEMVER.test(last)) throw new Error('기준이 될 정식 태그(vX.Y.Z)가 없습니다')
  const commits = commitsSince(last)
  const bump = forced === 'auto' ? decideBump(commits) : commits.length ? forced : null
  const result = { release: !!bump, bump, last, next: null, notes: null, commits: commits.length }
  if (bump) {
    result.next = bumpVersion(last, bump)
    result.notes = releaseNotes(result.next, commits)
  }
  if (json) process.stdout.write(JSON.stringify(result) + '\n')
  else if (!bump) console.log(`${last} 이후 커밋 ${commits.length}개 — 릴리스할 변경(feat·fix·perf·style) 없음`)
  else console.log(`${last} → v${result.next} (${bump}, 커밋 ${commits.length}개)\n\n${result.notes}`)
}

// 테스트가 import 할 때는 실행하지 않는다
if (/next-version\.mjs$/.test(process.argv[1] ?? '')) main()
