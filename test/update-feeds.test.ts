// Tauri 업데이터 피드(latest.json)와 서명 빌드 설정을 만드는 스크립트의 순수 함수.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { describe, it } from 'node:test'
import { PLATFORM, buildFeed, installerName, installerUrl, normalizeSignature } from '../scripts/update-feeds.mjs'
import { DEFAULT_ENDPOINT, buildUpdaterConfig } from '../scripts/updater-config.mjs'
import { parsePublicKey, parseSignature, signedVersion, verifyUpdateSignature } from '../scripts/updater-sig.mjs'

// 시험 전용 키 쌍의 공개 값(개인키는 저장소에 없다). src-tauri/src/updater.rs 시험도 같은 파일을 쓴다.
const fixture = (name: string) => JSON.parse(readFileSync(new URL(`./fixtures/${name}`, import.meta.url), 'utf8'))
const signing = fixture('updater-signing.json') as { pubkey: string; payload: string; signatureV999: string; signatureNoVersion: string }

const SIGNATURE = 'dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZQ=='
const PUBKEY = 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDZENDYyOTQxMzg5RTczNUIK'

describe('update-feeds', () => {
  it('설치본 이름과 URL 은 릴리스 자산 규칙을 따른다', () => {
    assert.equal(installerName('1.2.3'), 'Peekaboo-Setup-1.2.3-x64.exe')
    assert.equal(
      installerUrl('1.2.3'),
      'https://github.com/Kang-Jacob-GitLB/peekaboo/releases/download/v1.2.3/Peekaboo-Setup-1.2.3-x64.exe'
    )
  })

  it('latest.json 은 버전·노트·날짜·플랫폼별 서명과 URL 을 담는다', () => {
    const feed = buildFeed({ version: '1.2.3', notes: '고침', signature: `${SIGNATURE}\n`, pubDate: new Date('2026-11-16T00:00:00Z') })
    assert.deepEqual(feed, {
      version: '1.2.3',
      notes: '고침',
      pub_date: '2026-11-16T00:00:00.000Z',
      platforms: {
        [PLATFORM]: { signature: SIGNATURE, url: installerUrl('1.2.3') }
      }
    })
    assert.equal(PLATFORM, 'windows-x86_64')
  })

  it('baseUrl 이 있으면 설치본 주소만 바뀐다(로컬 리허설)', () => {
    const feed = buildFeed({ version: '1.2.3', signature: SIGNATURE, baseUrl: 'http://127.0.0.1:8123/' })
    assert.equal(feed.platforms[PLATFORM].url, 'http://127.0.0.1:8123/Peekaboo-Setup-1.2.3-x64.exe')
  })

  it('노트가 비면 버전으로 채운다', () => {
    assert.equal(buildFeed({ version: '1.0.0', signature: SIGNATURE }).notes, 'Peekaboo 1.0.0')
    assert.equal(buildFeed({ version: '1.0.0', notes: '  ', signature: SIGNATURE }).notes, 'Peekaboo 1.0.0')
  })

  it('서명이 비었거나 base64 가 아니면 실패한다', () => {
    for (const bad of ['', '  \n', undefined, null, 'not base64!', '주석: 서명']) {
      assert.throws(() => buildFeed({ version: '1.0.0', signature: bad as string }), Error, String(bad))
    }
    assert.throws(() => normalizeSignature(''), /비어/)
  })

  it('버전이 semver 가 아니면 실패한다', () => {
    for (const bad of ['', 'v1.0.0', '1.0', 'latest']) {
      assert.throws(() => buildFeed({ version: bad, signature: SIGNATURE }), /버전/, bad)
    }
    // 프리릴리스 꼬리표는 허용(피드는 만들 수 있다. releases/latest 가 프리릴리스를 제외한다)
    assert.equal(buildFeed({ version: '1.0.0-beta.1', signature: SIGNATURE }).version, '1.0.0-beta.1')
  })
})

describe('updater-config', () => {
  it('공개키와 기본 엔드포인트로 서명 빌드 설정을 만든다', () => {
    const config = buildUpdaterConfig({ pubkey: `${PUBKEY}\n` })
    assert.deepEqual(config, {
      plugins: {
        updater: {
          pubkey: PUBKEY,
          endpoints: [DEFAULT_ENDPOINT],
          windows: { installMode: 'passive' },
          requireSignedVersion: true
        }
      }
    })
    assert.equal(DEFAULT_ENDPOINT, 'https://github.com/Kang-Jacob-GitLB/peekaboo/releases/latest/download/latest.json')
  })

  it('서명 산출물은 요청할 때만 만든다(CI 빌드는 개인키가 없다)', () => {
    assert.equal('bundle' in buildUpdaterConfig({ pubkey: PUBKEY }), false)
    assert.deepEqual(buildUpdaterConfig({ pubkey: PUBKEY, createArtifacts: true }).bundle, { createUpdaterArtifacts: true })
  })

  it('플러그인 설정은 앱 시험(updater.rs)이 쓰는 기준 파일과 같다', () => {
    const { _comment, ...base } = fixture('updater-plugin.json')
    assert.ok(_comment)
    assert.deepEqual(buildUpdaterConfig({ pubkey: 'PUBKEY', endpoint: 'http://127.0.0.1:0/latest.json' }).plugins.updater, base)
  })

  it('http 엔드포인트(로컬 리허설)에서만 비보안 전송을 허용한다', () => {
    const local = buildUpdaterConfig({ pubkey: PUBKEY, endpoint: 'http://127.0.0.1:8080/latest.json' })
    assert.equal(local.plugins.updater.dangerousInsecureTransportProtocol, true)
    const secure = buildUpdaterConfig({ pubkey: PUBKEY, endpoint: 'https://example.com/latest.json' })
    assert.equal('dangerousInsecureTransportProtocol' in secure.plugins.updater, false)
  })

  it('공개키가 비었거나 base64 가 아니거나 주소가 이상하면 실패한다', () => {
    assert.throws(() => buildUpdaterConfig({ pubkey: '' }), /비어/)
    assert.throws(() => buildUpdaterConfig({ pubkey: 'untrusted comment: x' }), /base64/)
    assert.throws(() => buildUpdaterConfig({ pubkey: PUBKEY, endpoint: 'ftp://x/latest.json' }), /주소/)
    assert.throws(() => buildUpdaterConfig({ pubkey: PUBKEY, endpoint: 'nope' }))
  })
})

describe('updater-sig', () => {
  const installer = Buffer.from(signing.payload)
  const args = { installer, signature: signing.signatureV999, pubkey: signing.pubkey, version: '9.9.9' }

  /** 공개키 본문(42바이트)을 고쳐 다시 감싼 공개키 */
  const tamperedPubkey = (offset: number) => {
    const lines = Buffer.from(signing.pubkey, 'base64').toString().split('\n')
    const body = Buffer.from(lines[1], 'base64')
    body[offset] ^= 1
    return Buffer.from(`${lines[0]}\n${body.toString('base64')}\n`).toString('base64')
  }

  it('공개키와 서명의 구성 요소를 읽는다', () => {
    const pub = parsePublicKey(signing.pubkey)
    const sig = parseSignature(signing.signatureV999)
    assert.equal(pub.key.length, 32)
    assert.equal(pub.keyId.length, 8)
    assert.equal(sig.algorithm, 'ED')
    assert.ok(pub.keyId.equals(sig.keyId))
    assert.equal(signedVersion(sig.trustedComment), '9.9.9')
  })

  it('맞는 설치본·키·버전이면 통과한다', () => {
    verifyUpdateSignature(args)
  })

  it('설치본이 바뀌면 거부한다', () => {
    assert.throws(() => verifyUpdateSignature({ ...args, installer: Buffer.from('tampered-payload') }), /설치본과 맞지 않/)
  })

  it('키 ID 가 다른 공개키면 거부한다', () => {
    assert.throws(() => verifyUpdateSignature({ ...args, pubkey: tamperedPubkey(2) }), /키 ID/)
  })

  it('키 ID 는 같아도 공개키 값이 다르면 거부한다', () => {
    assert.throws(() => verifyUpdateSignature({ ...args, pubkey: tamperedPubkey(20) }), /설치본과 맞지 않/)
  })

  it('릴리스 버전과 서명이 덮는 버전이 다르면 거부한다(부분 문자열 일치도 안 된다)', () => {
    for (const version of ['9.9.8', '9.9.90', '9.9', '9.9.9-beta']) {
      assert.throws(() => verifyUpdateSignature({ ...args, version }), /서명이 덮는 버전/, version)
    }
  })

  it('버전이 없는 서명은 거부한다', () => {
    assert.throws(() => verifyUpdateSignature({ ...args, signature: signing.signatureNoVersion }), /version:/)
  })

  it('trusted comment 를 바꾸면 전역 서명이 어긋나 거부한다', () => {
    const text = Buffer.from(signing.signatureV999, 'base64').toString().replace('version:9.9.9', 'version:9.9.8')
    assert.throws(() => verifyUpdateSignature({ ...args, signature: Buffer.from(text).toString('base64'), version: '9.9.8' }), /변조/)
  })

  it('형식이 아닌 입력은 거부한다', () => {
    assert.throws(() => parsePublicKey('AAAA'), Error)
    assert.throws(() => parseSignature('not base64!'), /base64/)
  })
})
