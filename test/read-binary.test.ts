// 오피스/PDF 플러그인이 쓰는 파일 전체 읽기(read-binary): 4MiB 단위 Range 로 미리 할당한 버퍼를 채운다.

import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { CHUNK_BYTES, MAX_BYTES, readBinary, readWhole } from '../src/renderer/plugins/read-binary.ts'
import { createFakeFetch } from './helpers/fake-fetch.ts'

const MIB = 1024 * 1024
/** 위치에 따라 값이 달라지는 데이터(조각이 어긋나면 비교에서 드러난다) */
const pattern = (n: number): Buffer => Buffer.from(Array.from({ length: n }, (_, i) => (i * 31 + (i >> 8)) & 0xff))

const setup = (data: Buffer, mode: Parameters<typeof createFakeFetch>[0] = 'range') => {
  const fake = createFakeFetch(mode, new Map([['f.bin', data]]))
  return { fake, url: fake.fileUrl('f.bin') }
}

describe('readWhole', () => {
  it('4MiB 조각으로 읽고 요청 수가 조각 수와 같다(전체 GET 없음, 크기 요청 없음)', async () => {
    const data = pattern(10 * MIB + 123)
    const { fake, url } = setup(data)
    const out = await readWhole(url, fake.fetch)
    assert.equal(out.byteLength, data.length)
    assert.equal(Buffer.compare(Buffer.from(out), data), 0)
    assert.deepEqual(fake.requests, [
      `bytes=0-${CHUNK_BYTES - 1}`,
      `bytes=${CHUNK_BYTES}-${2 * CHUNK_BYTES - 1}`,
      `bytes=${2 * CHUNK_BYTES}-${data.length - 1}`
    ])
  })

  it('조각 크기의 정확한 배수도 요청이 남지 않는다', async () => {
    const data = pattern(2 * CHUNK_BYTES)
    const { fake, url } = setup(data)
    const out = await readWhole(url, fake.fetch)
    assert.equal(Buffer.compare(Buffer.from(out), data), 0)
    assert.equal(fake.requests.length, 2)
  })

  it('한 조각보다 작은 파일은 요청 한 번이다', async () => {
    const data = pattern(1000)
    const { fake, url } = setup(data)
    const out = await readWhole(url, fake.fetch)
    assert.equal(Buffer.compare(Buffer.from(out), data), 0)
    assert.equal(fake.requests.length, 1)
  })

  it('빈 파일은 빈 버퍼', async () => {
    const { fake, url } = setup(Buffer.alloc(0))
    const out = await readWhole(url, fake.fetch)
    assert.equal(out.byteLength, 0)
  })

  it('상한을 넘으면 첫 조각만 받고 throw 한다', async () => {
    const { fake, url } = setup(pattern(5000))
    await assert.rejects(readWhole(url, fake.fetch, 4999, 1024), /100MB/)
    assert.equal(fake.requests.length, 1)
    // 상한과 같으면 읽는다
    const same = setup(pattern(5000))
    const ok = await readWhole(same.url, same.fake.fetch, 5000, 1024)
    assert.equal(ok.byteLength, 5000)
  })

  it('응답이 요청보다 짧게 와도(응답 크기 상한) 이어 받아 같은 결과', async () => {
    const data = pattern(5000)
    const { fake, url } = setup(data, 'chunked')
    const out = await readWhole(url, fake.fetch, MAX_BYTES, 1024)
    assert.equal(Buffer.compare(Buffer.from(out), data), 0)
  })

  for (const mode of ['ignore', 'ignore-nolen'] as const) {
    it(`Range 를 무시하는 서버(${mode})는 요청 한 번에 전체를 받고 전송량이 파일 크기 수준이다`, async () => {
      const data = pattern(5 * MIB + 7)
      const { fake, url } = setup(data, mode)
      const out = await readWhole(url, fake.fetch)
      assert.equal(Buffer.compare(Buffer.from(out), data), 0)
      assert.equal(fake.requests.length, 1)
      assert.equal(fake.bytesSent, data.length)
    })

    it(`Range 를 무시하는 서버(${mode})도 상한을 넘으면 throw 하고 전체를 받지 않는다`, async () => {
      const { fake, url } = setup(pattern(1 * MIB), mode)
      await assert.rejects(readWhole(url, fake.fetch, 100 * 1024, 1024), /100MB/)
      assert.equal(fake.requests.length, 1)
      assert.ok(fake.bytesSent < 1 * MIB, `전송량 ${fake.bytesSent}`)
    })
  }

  it('읽는 도중 파일이 줄어 끝이 모자라면 throw 한다', async () => {
    const data = pattern(4000)
    const { fake, url } = setup(data)
    let calls = 0
    const shrinking = async (u: string, init?: RequestInit) => {
      if (++calls === 1) return fake.fetch(u, init)
      return new Response(null, { status: 416, headers: { 'Content-Range': 'bytes */1500' } })
    }
    await assert.rejects(readWhole(url, shrinking, MAX_BYTES, 1024), /읽을 수 없습니다/)
  })

  it('요청이 실패하면 throw 한다', async () => {
    const failing = async () => new Response(null, { status: 500 })
    await assert.rejects(readWhole('x', failing), /읽을 수 없습니다/)
  })

  it('중단 신호가 오면 다음 조각을 받지 않는다', async () => {
    const data = pattern(5000)
    const { fake, url } = setup(data)
    const ac = new AbortController()
    let calls = 0
    const f = async (u: string, init?: RequestInit) => {
      if (++calls === 2) ac.abort()
      if (ac.signal.aborted) throw new DOMException('aborted', 'AbortError')
      return fake.fetch(u, init)
    }
    await assert.rejects(readWhole(url, f, MAX_BYTES, 1024), /abort/i)
    assert.equal(calls, 2)
  })
})

describe('readBinary', () => {
  it('100MB 를 넘는 파일은 요청 없이 throw 한다', async () => {
    const ctx = {
      file: { size: MAX_BYTES + 1, path: 'p' },
      host: {
        fileUrl: () => {
          throw new Error('호출되면 안 된다')
        }
      },
      signal: new AbortController().signal
    }
    await assert.rejects(readBinary(ctx), /100MB/)
  })
})
