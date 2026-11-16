// 업데이터 서명(minisign) 검증. `tauri signer sign` 이 만든 설치본의 .sig 를 공개키(src-tauri/updater.pub)로 확인한다.
// 앱(tauri-plugin-updater)이 받을 때 하는 검증을 릴리스 쪽에서 미리 해서, 키가 어긋났거나 버전이 틀린 서명이 피드에 실리지 않게 한다.
//
// 형식(minisign)
//   공개키 파일(.pub) = base64( "untrusted comment: …\n" + base64(`Ed` + 키 ID 8바이트 + Ed25519 공개키 32바이트) + "\n" )
//   서명 파일(.sig)   = base64( "untrusted comment: …\n" + base64(알고리즘 2바이트 + 키 ID 8바이트 + 서명 64바이트) + "\n"
//                              + "trusted comment: …\n" + base64(전역 서명 64바이트) + "\n" )
//   알고리즘 `ED` 는 파일 전체가 아니라 BLAKE2b-512 해시에 서명한 것(프리해시)이고, `Ed` 는 파일 자체에 서명한 것이다.
//   전역 서명은 `서명 64바이트 ‖ trusted comment 바이트` 에 대한 Ed25519 서명이라 trusted comment(여기에 `version:` 이 든다)를 보호한다.
// Node 의 crypto 가 Ed25519 와 blake2b512 를 모두 제공해 외부 도구 없이 전부 검증한다.
import { createHash, createPublicKey, verify } from 'node:crypto'

// Ed25519 공개키(32바이트) 앞에 붙이는 SubjectPublicKeyInfo DER 머리
const SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex')

function decodeBase64(text, what) {
  const clean = String(text ?? '').replace(/\s+/g, '')
  if (clean === '' || !/^[A-Za-z0-9+/]+={0,2}$/.test(clean)) throw new Error(`${what}이(가) base64 가 아닙니다`)
  return Buffer.from(clean, 'base64')
}

/** 바깥 base64 를 벗겨 minisign 텍스트의 줄 목록으로 */
function outerLines(base64, what) {
  return decodeBase64(base64, what).toString('utf8').split(/\r?\n/)
}

/** 공개키 파일 내용(base64 한 줄) → { keyId(8바이트), key(32바이트) } */
export function parsePublicKey(pubkey) {
  const lines = outerLines(pubkey, '공개키')
  const body = decodeBase64(lines[1], '공개키 본문')
  if (body.length !== 42 || body.toString('latin1', 0, 2) !== 'Ed') throw new Error('공개키 형식이 minisign 이 아닙니다')
  return { keyId: body.subarray(2, 10), key: body.subarray(10, 42) }
}

/** .sig 내용(base64) → 구성 요소. 서명 검증은 하지 않는다. */
export function parseSignature(signature) {
  const lines = outerLines(signature, '서명')
  const body = decodeBase64(lines[1], '서명 본문')
  if (body.length !== 74) throw new Error('서명 형식이 minisign 이 아닙니다')
  const algorithm = body.toString('latin1', 0, 2)
  if (algorithm !== 'ED' && algorithm !== 'Ed') throw new Error(`알 수 없는 서명 알고리즘: ${algorithm}`)
  const trusted = lines[2] ?? ''
  if (!trusted.startsWith('trusted comment: ')) throw new Error('서명에 trusted comment 가 없습니다')
  return {
    algorithm,
    keyId: body.subarray(2, 10),
    signature: body.subarray(10, 74),
    trustedComment: trusted.slice('trusted comment: '.length),
    globalSignature: decodeBase64(lines[3], '전역 서명')
  }
}

/** trusted comment(탭으로 구분한 `키:값`)에서 `version:` 필드. 부분 문자열이 아니라 필드 전체가 같아야 한다. */
export function signedVersion(trustedComment) {
  for (const field of String(trustedComment).split('\t')) {
    if (field.startsWith('version:')) return field.slice('version:'.length)
  }
  return undefined
}

/** Ed25519 서명 검증(Node crypto) */
function ed25519(key, message, signature) {
  const publicKey = createPublicKey({ key: Buffer.concat([SPKI_PREFIX, key]), format: 'der', type: 'spki' })
  return verify(null, message, publicKey, signature)
}

/**
 * 설치본의 서명을 공개키로 검증하고, 서명이 덮는 버전이 기대한 버전과 같은지 확인한다. 하나라도 어긋나면 던진다.
 * `version:` 은 전역 서명이 맞은 뒤에만 믿는다(그 전의 trusted comment 는 공격자가 바꿀 수 있다).
 * @param {{installer: Uint8Array, signature: string, pubkey: string, version: string}} input
 *   signature: .sig 파일 내용(base64), pubkey: updater.pub 내용(base64)
 */
export function verifyUpdateSignature({ installer, signature, pubkey, version }) {
  const pub = parsePublicKey(pubkey)
  const sig = parseSignature(signature)
  if (!pub.keyId.equals(sig.keyId)) {
    throw new Error(
      `서명의 키 ID(${sig.keyId.toString('hex')})가 공개키(src-tauri/updater.pub)의 키 ID(${pub.keyId.toString('hex')})와 다릅니다. 다른 키로 서명했거나 updater.pub 가 낡았습니다`
    )
  }
  const message = sig.algorithm === 'ED' ? createHash('blake2b512').update(installer).digest() : Buffer.from(installer)
  if (!ed25519(pub.key, message, sig.signature)) throw new Error('서명이 설치본과 맞지 않습니다(설치본이 바뀌었거나 다른 파일의 서명입니다)')
  const trusted = Buffer.concat([sig.signature, Buffer.from(sig.trustedComment, 'utf8')])
  if (!ed25519(pub.key, trusted, sig.globalSignature)) throw new Error('서명의 trusted comment 가 변조됐습니다')
  const signed = signedVersion(sig.trustedComment)
  if (signed === undefined) {
    throw new Error('서명에 버전(version:)이 없습니다. signer sign 에 --app-version 을 주세요(앱은 requireSignedVersion 으로 이를 요구합니다)')
  }
  if (signed !== version) throw new Error(`서명이 덮는 버전(${signed})이 릴리스 버전(${version})과 다릅니다`)
}
