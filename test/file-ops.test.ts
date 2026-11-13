// 골든 비교로 덮이지 않는 부분: range-reader 의 방어, 상한 동작, 블록 캐시, 바이트 도구, Range 헤더 해석.

process.env.TZ = 'UTC'

import assert from 'node:assert/strict'
import { crc32 as nodeCrc32, gzipSync } from 'node:zlib'
import { mkdtemp, open, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, it } from 'node:test'
import { partialHeaders, rangeBody, unsatisfiableHeaders } from '../src/main/peek-response.ts'
import { parseRange } from '../src/main/range.ts'
import { DEFAULT_CAPS, listArchive } from '../src/renderer/host/file-ops/archive.ts'
import { crc32, latin1, utf8 } from '../src/renderer/host/file-ops/bytes.ts'
import { createFileOps } from '../src/renderer/host/file-ops/index.ts'
import { createRangeReader } from '../src/renderer/host/file-ops/range-reader.ts'
import { buildFixtures, tarHeader } from './helpers/fixtures.ts'
import { MODES, createFakeFetch } from './helpers/fake-fetch.ts'

const fixtures = buildFixtures()
/** 디스크 없이 메모리의 파일을 서빙하는 fake. 경로는 임의 문자열 */
const mem = (name: string, data: Buffer, mode = MODES[0]) => {
  const fake = createFakeFetch(mode, new Map([[name, data]]))
  return { fake, reader: createRangeReader(fake.fileUrl(name), fake.fetch) }
}

describe('바이트 도구', () => {
  it('crc32 는 node:zlib 와 같다', () => {
    for (const s of ['', 'a', 'hello world', '한글 이름.txt']) assert.equal(crc32(Buffer.from(s)), nodeCrc32(Buffer.from(s)))
    const rnd = Buffer.from(Array.from({ length: 5000 }, (_, i) => (i * 7919 + 13) % 256))
    assert.equal(crc32(rnd), nodeCrc32(rnd))
  })
  it('latin1 은 0x80~0x9F 도 그대로 한 글자다(windows-1252 가 아니다)', () => {
    assert.equal(latin1(Buffer.from([0x61, 0x80, 0x9f, 0xff])), 'a\u0080\u009f\u00ff')
    assert.equal(latin1(Buffer.alloc(20000, 0x41)).length, 20000)
  })
  it('utf8 는 앞의 BOM 을 지우지 않는다', () => {
    assert.equal(utf8(Buffer.from([0xef, 0xbb, 0xbf, 0x61])), '\ufeffa')
  })
})

describe('range-reader', () => {
  it('Content-Range 시작이 요청과 다르면 거부한다', async () => {
    const r = createRangeReader('x://f', async () => new Response(Buffer.alloc(10), { status: 206, headers: { 'Content-Range': 'bytes 5-14/100' } }))
    await assert.rejects(r.read(0, 10), /파일을 읽을 수 없습니다/)
  })
  it('Content-Range 가 없는 206 은 거부한다', async () => {
    const r = createRangeReader('x://f', async () => new Response(Buffer.alloc(10), { status: 206 }))
    await assert.rejects(r.read(0, 10), /파일을 읽을 수 없습니다/)
  })
  it('403 같은 오류 본문을 파일 바이트로 해석하지 않는다', async () => {
    const r = createRangeReader('x://f', async () => new Response('Forbidden', { status: 403 }))
    await assert.rejects(r.read(0, 10), /HTTP 403/)
    await assert.rejects(r.size(), /HTTP 403/)
  })
  it('빈 파일(416)은 크기 0 이다', async () => {
    const { reader } = mem('e', Buffer.alloc(0))
    assert.equal(await reader.size(), 0)
    assert.equal((await reader.read(0, 10)).length, 0)
  })
  it('파일 끝을 넘는 구간은 짧게 돌려준다', async () => {
    for (const mode of MODES) {
      const { reader } = mem('f', Buffer.from('0123456789'), mode)
      assert.equal(Buffer.from(await reader.read(7, 100)).toString(), '789', mode)
      assert.equal((await reader.read(50, 10)).length, 0, mode)
      assert.equal(await reader.size(), 10, mode)
    }
  })
  it('응답이 잘려 와도(chunked) 이어 받아 요청한 길이를 채운다', async () => {
    const data = Buffer.from(Array.from({ length: 5000 }, (_, i) => i % 251))
    const { reader, fake } = mem('f', data, 'chunked')
    assert.deepEqual(Buffer.from(await reader.read(100, 3000)), data.subarray(100, 3100))
    assert.ok(fake.requests.length >= 5)
  })
  it('200 응답은 필요한 만큼만 읽고 크기를 Content-Length 에서 얻는다', async () => {
    let cancelled = false
    const body = new ReadableStream<Uint8Array>({
      pull(c) {
        c.enqueue(new Uint8Array(1000).fill(7))
      },
      cancel() {
        cancelled = true
      }
    })
    const r = createRangeReader('x://f', async () => new Response(body, { status: 200, headers: { 'Content-Length': '1000000000' } }))
    assert.equal((await r.read(0, 1500)).length, 1500)
    assert.equal(await r.size(), 1_000_000_000)
    assert.ok(cancelled)
  })
})

describe('텍스트', () => {
  it('maxBytes 가 16MB 를 넘어도 16MB 에서 자른다', async () => {
    const data = Buffer.alloc(17 * 1024 * 1024, 0x61)
    const fake = createFakeFetch('range', new Map([['big.txt', data]]))
    const res = await createFileOps(fake.fileUrl, fake.fetch).readText('big.txt', 100 * 1024 * 1024)
    assert.equal(res.text.length, 16 * 1024 * 1024)
    assert.equal(res.truncated, true)
  })
})

describe('압축 목록 상한', () => {
  it('중앙 디렉터리 상한을 넘으면 앞부분만 보이고 scanLimited 가 켜진다', async () => {
    const { reader } = mem('many.zip', fixtures['many.zip'])
    const res = await listArchive(reader, 'many.zip', 1000, { ...DEFAULT_CAPS, zipMaxCdBytes: 300 })
    assert.equal(res.scanLimited, true)
    assert.equal(res.truncated, true)
    assert.ok(res.entries.length > 0 && res.entries.length < 60)
    assert.equal(res.totalEntries, 60)
  })
  it('항목 수는 100000 으로 제한한다', async () => {
    const { reader } = mem('many.zip', fixtures['many.zip'])
    assert.equal((await listArchive(reader, 'many.zip', Number.POSITIVE_INFINITY)).entries.length, 0)
    assert.equal((await listArchive(reader, 'many.zip', 1e9)).entries.length, 60)
  })
  it('tar.gz 해제 상한을 넘으면 scanLimited 가 켜진다', async () => {
    const body = Buffer.concat([tarHeader({ name: 'big.bin', size: 20_000_000 }), Buffer.alloc(20_000_000)])
    const tail = Buffer.concat([tarHeader({ name: 'after.txt', size: 1 }), Buffer.alloc(512), Buffer.alloc(1024)])
    const { reader } = mem('x.tar.gz', gzipSync(Buffer.concat([body, tail])))
    const res = await listArchive(reader, 'x.tar.gz', 100, { ...DEFAULT_CAPS, gzMaxScanBytes: 1_000_000 })
    assert.equal(res.scanLimited, true)
    assert.equal(res.truncated, true)
    assert.equal(res.entries.length, 1)
  })
  it('평문 tar 는 헤더가 연달아 있으면 선읽기를 키워 요청 수를 줄인다', async () => {
    const blocks: Buffer[] = []
    for (let i = 0; i < 2000; i++) blocks.push(tarHeader({ name: `f${i}.txt`, size: 0 }))
    blocks.push(Buffer.alloc(1024))
    const data = Buffer.concat(blocks) // 약 1MB
    const { reader, fake } = mem('many.tar', data)
    const res = await listArchive(reader, 'many.tar', 5000)
    assert.equal(res.entries.length, 2000)
    // 선읽기 16KB 에서 시작해 두 배씩(상한 256KB) 커지므로 수십 번이면 충분하다. 512바이트씩 읽었다면 2000번이다
    assert.ok(fake.requests.length <= 20, `요청 ${fake.requests.length}개`)
    assert.ok(fake.bytesSent <= data.length + 512 * 1024, `전송 ${fake.bytesSent}바이트`)
  })
  it('평문 tar 는 본문을 건너뛰고 항목당 수십 KB 이하만 전송한다', async () => {
    const parts: Buffer[] = []
    const COUNT = 6
    for (let i = 0; i < COUNT; i++) parts.push(tarHeader({ name: `big${i}.bin`, size: 300_000 }), Buffer.alloc(Math.ceil(300_000 / 512) * 512))
    parts.push(Buffer.alloc(1024))
    const data = Buffer.concat(parts)
    const { reader, fake } = mem('big.tar', data)
    const res = await listArchive(reader, 'big.tar', 100)
    assert.equal(res.entries.length, COUNT)
    assert.equal(res.entries[COUNT - 1].path, `big${COUNT - 1}.bin`)
    // 항목당 헤더 512 + 선읽기 16KB 수준. 256KB 블록 정렬이었다면 항목당 256KB 였다
    assert.ok(fake.bytesSent <= COUNT * 40 * 1024, `전송 ${fake.bytesSent}바이트`)
    assert.ok(fake.bytesSent < data.length / 10)
  })
})

describe('range-reader 의 Range 무시(200) 폴백', () => {
  it('큰 오프셋을 읽어도 필요한 구간만 정확히 돌려주고 뒤는 받지 않는다', async () => {
    const data = Buffer.from(Array.from({ length: 4_000_000 }, (_, i) => (i * 31 + 7) % 251))
    for (const mode of ['ignore', 'ignore-nolen'] as const) {
      const { reader, fake } = mem('big.bin', data, mode)
      const got = await reader.read(1_000_000, 100)
      assert.equal(got.length, 100, mode)
      assert.ok(Buffer.from(got).equals(data.subarray(1_000_000, 1_000_100)), mode)
      // 길이를 알면 필요한 곳까지만 받고 닫는다
      if (mode === 'ignore') assert.ok(fake.bytesSent < 1_000_100 + 2 * 65536, `전송 ${fake.bytesSent}바이트`)
      assert.equal(await reader.size(), data.length, mode)
    }
  })
  it('파일 끝을 넘는 요청은 짧게 돌려준다', async () => {
    const { reader } = mem('s.bin', Buffer.from('0123456789'), 'ignore')
    assert.equal(Buffer.from(await reader.read(8, 100)).toString(), '89')
    assert.equal((await reader.read(20, 5)).length, 0)
  })
})

describe('peek 응답 조립(main/peek-response.ts)', () => {
  it('206 헤더: Content-Range·Content-Length·Content-Type 과 노출 헤더', () => {
    const h = partialHeaders(10, 19, 100, 'text/plain')
    assert.equal(h.get('content-range'), 'bytes 10-19/100')
    assert.equal(h.get('content-length'), '10')
    assert.equal(h.get('content-type'), 'text/plain')
    assert.equal(h.get('access-control-allow-origin'), '*')
    assert.match(h.get('access-control-expose-headers') ?? '', /Content-Range/)
    assert.equal(h.get('accept-ranges'), 'bytes')
  })
  it('416 헤더: bytes */크기 와 노출 헤더', () => {
    const h = unsatisfiableHeaders(42)
    assert.equal(h.get('content-range'), 'bytes */42')
    assert.match(h.get('access-control-expose-headers') ?? '', /Content-Range/)
    assert.equal(h.get('access-control-allow-origin'), '*')
  })
  it('Content-Type 이 없으면 헤더를 만들지 않는다', () => {
    assert.equal(partialHeaders(0, 0, 1, null).has('content-type'), false)
  })
  it('rangeBody 는 구간만 읽고 끝나면 핸들을 닫는다', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'peek-'))
    try {
      const file = join(dir, 'a.bin')
      await writeFile(file, Buffer.from('0123456789abcdef'))
      const fh = await open(file, 'r')
      const body = await new Response(rangeBody(fh, 4, 9)).text()
      assert.equal(body, '456789')
      await new Promise((r) => setTimeout(r, 20))
      assert.equal(fh.fd, -1, '핸들이 닫혀야 한다')
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })
  it('rangeBody 는 취소해도 핸들을 닫는다', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'peek-'))
    try {
      const file = join(dir, 'b.bin')
      await writeFile(file, Buffer.alloc(1_000_000, 1))
      const fh = await open(file, 'r')
      const rd = rangeBody(fh, 0, 999_999).getReader()
      await rd.read()
      await rd.cancel()
      await new Promise((r) => setTimeout(r, 50))
      assert.equal(fh.fd, -1, '핸들이 닫혀야 한다')
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })
})

describe('Range 헤더 해석(main/range.ts)', () => {
  const cases: [string | null, number, unknown][] = [
    [null, 100, { kind: 'none' }],
    ['bytes=0-9', 100, { kind: 'ok', start: 0, end: 9 }],
    ['bytes=90-', 100, { kind: 'ok', start: 90, end: 99 }],
    ['bytes=90-500', 100, { kind: 'ok', start: 90, end: 99 }],
    ['bytes=-10', 100, { kind: 'ok', start: 90, end: 99 }],
    ['bytes=-500', 100, { kind: 'ok', start: 0, end: 99 }],
    ['bytes=100-', 100, { kind: 'unsatisfiable' }],
    ['bytes=0-0', 0, { kind: 'unsatisfiable' }],
    ['bytes=-0', 100, { kind: 'unsatisfiable' }],
    ['bytes=-5', 0, { kind: 'unsatisfiable' }],
    ['bytes=9-0', 100, { kind: 'none' }],
    ['bytes=0-9,20-29', 100, { kind: 'none' }],
    ['bytes=-', 100, { kind: 'none' }],
    ['items=0-9', 100, { kind: 'none' }],
    ['bytes=a-b', 100, { kind: 'none' }],
    ['bytes=99999999999999999999-', 100, { kind: 'none' }]
  ]
  for (const [header, size, want] of cases) {
    it(`${header} (크기 ${size})`, () => assert.deepEqual(parseRange(header, size), want))
  }
})
