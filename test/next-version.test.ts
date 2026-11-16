// 야간 릴리스의 다음 버전·노트 계산(scripts/next-version.mjs)의 순수 함수.
import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { RELEASE_NOTES_TAIL, bumpVersion, classify, decideBump, releaseNotes } from '../scripts/next-version.mjs'

const commit = (subject: string, body = '') => ({ hash: 'x', subject, body })

describe('next-version', () => {
  it('커밋 제목으로 올림 단계를 정한다', () => {
    assert.equal(decideBump([commit('docs: 문서'), commit('fix: 버그')]), 'patch')
    assert.equal(decideBump([commit('fix: 버그'), commit('feat: 기능')]), 'minor')
    assert.equal(decideBump([commit('feat!: 전환')]), 'major')
    assert.equal(decideBump([commit('chore: x', 'BREAKING CHANGE: 이전 필요')]), 'major')
    assert.equal(decideBump([commit('ci: x'), commit('docs: y')]), null)
  })

  it('버전을 올린다', () => {
    assert.equal(bumpVersion('v0.9.3', 'major'), '1.0.0')
    assert.equal(bumpVersion('v1.2.3', 'minor'), '1.3.0')
    assert.equal(bumpVersion('v1.2.3', 'patch'), '1.2.4')
  })

  it('느낌표 타입은 호환성이 바뀌는 변경이다', () => {
    assert.equal(classify('feat!: Tauri 2 로 전환').breaking, true)
  })

  it('릴리스 노트 꼬리에 Tauri 업데이트와 0.x 이전 안내가 있다', () => {
    const notes = releaseNotes('1.0.0', [commit('feat!: Tauri 2 로 전환')])
    assert.ok(notes.startsWith('v1.0.0\n'))
    assert.ok(notes.includes('## 호환성이 바뀌는 변경'))
    assert.ok(notes.endsWith(RELEASE_NOTES_TAIL))
    assert.match(RELEASE_NOTES_TAIL, /Tauri 버전\(1\.0\.0 베타 포함\)/)
    assert.match(RELEASE_NOTES_TAIL, /다시 시작하여 업데이트/)
    assert.match(RELEASE_NOTES_TAIL, /0\.x\(Electron\)/)
    assert.match(RELEASE_NOTES_TAIL, /직접 한 번 설치/)
    assert.doesNotMatch(RELEASE_NOTES_TAIL, /0\.2\.0/)
  })
})
