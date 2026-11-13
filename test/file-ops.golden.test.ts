// 골든 비교: 렌더러의 파일 해석(host/file-ops)이 예전 main 의 Node 구현과 같은 출력을 내는지 본다.
// 기대값(test/fixtures/expected/*.json)은 88fd827 의 file-service.ts·tar-list.ts 에 TZ=UTC 로 같은 픽스처를 돌려 얻었다.
// 기대값을 바꾸는 것은 해석 규칙을 바꾸는 일이므로 보안 리뷰 대상이다.

process.env.TZ = 'UTC' // zip 의 MS-DOS 시각은 로컬 시간대로 해석된다

import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { after, before, describe, it } from 'node:test'
import { createFileOps } from '../src/renderer/host/file-ops/index.ts'
import { buildCases, buildFixtures } from './helpers/fixtures.ts'
import { MODES, createFakeFetch } from './helpers/fake-fetch.ts'

const expectedDir = join(import.meta.dirname, 'fixtures', 'expected')
let dir = ''
const fixtures = buildFixtures()
const cases = buildCases(Object.keys(fixtures))

before(async () => {
  dir = await mkdtemp(join(tmpdir(), 'peekaboo-test-'))
  for (const [name, bytes] of Object.entries(fixtures)) await writeFile(join(dir, name), bytes)
})
after(async () => {
  await rm(dir, { recursive: true, force: true })
})

for (const mode of MODES) {
  describe(`골든 비교 (서버 모드: ${mode})`, () => {
    for (const file of Object.keys(fixtures)) {
      it(file, async () => {
        const expected = JSON.parse(await readFile(join(expectedDir, `${file}.json`), 'utf8')) as Record<string, unknown>
        const fake = createFakeFetch(mode)
        const ops = createFileOps(fake.fileUrl, fake.fetch)
        for (const c of cases.filter((x) => x.file === file)) {
          const path = join(dir, c.file)
          let actual: unknown
          try {
            actual = c.kind === 'text' ? await ops.readText(path, c.arg) : await ops.listArchive(path, c.arg)
          } catch (e) {
            actual = { error: (e as Error).message }
          }
          // JSON 으로 한 번 왕복해 undefined 속성을 기대값(JSON)과 같은 조건으로 맞춘다
          assert.deepStrictEqual(JSON.parse(JSON.stringify(actual)), expected[String(c.arg)], `${file} @ ${c.arg}`)
        }
      })
    }
  })
}

it('모든 픽스처에 기대값 파일이 있다', async () => {
  for (const file of Object.keys(fixtures)) await readFile(join(expectedDir, `${file}.json`))
})
