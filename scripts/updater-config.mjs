// Tauri 빌드에 얹는 업데이터 설정(`tauri build --config <파일>`)을 만든다.
// 서명 공개키는 저장소의 `src-tauri/updater.pub` 파일에서만 읽는다(공개 값이라 커밋한다. 개인키는 절대 저장소에 두지 않는다).
// 파일이 없으면 업데이터 없는 빌드가 나간다. 설정이 있어야
//   - 앱이 업데이터 플러그인을 등록한다(공개키가 없으면 등록하지 않고 업데이트가 꺼진다: src-tauri/src/main.rs, updater.rs).
//   - `createUpdaterArtifacts` 로 설치본의 서명(.sig)이 만들어진다(개인키는 TAURI_SIGNING_PRIVATE_KEY). CI 는 빌드에서 개인키를 쓰지 않으므로
//     이것을 끄고(`createArtifacts: false`), 별도 sign 잡이 `tauri signer sign` 으로 서명한다(.github/workflows/tauri-build.yml).
// Windows 설치 모드는 passive: 진행 표시줄만 보이고 설치 뒤 앱을 다시 실행한다(`/P /UPDATE /R`, 근거는 docs/tauri-migration.md 6절).
import { existsSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

/** 업데이트 피드. releases/latest 는 프리릴리스를 제외하므로 정식 릴리스만 받는다. */
export const DEFAULT_ENDPOINT = 'https://github.com/Kang-Jacob-GitLB/peekaboo/releases/latest/download/latest.json'

/** 저장소 안의 공개키 파일(`tauri signer generate` 가 만든 .pub 파일 그대로) */
export const PUBKEY_FILE = join(dirname(fileURLToPath(import.meta.url)), '..', 'src-tauri', 'updater.pub')

/** 공개키 파일 내용(base64 한 줄). 파일이 없으면 null(업데이터 없는 빌드). */
export function readPubkey(file = PUBKEY_FILE) {
  return existsSync(file) ? readFileSync(file, 'utf8').trim() : null
}

/**
 * @param {{pubkey: string, endpoint?: string, createArtifacts?: boolean}} options
 *   pubkey: `tauri signer generate` 가 만든 .pub 파일의 내용(base64 한 줄)
 *   endpoint: 시험용으로 바꿀 피드 주소. http:// 이면(로컬 서버 리허설) 비보안 전송을 허용한다.
 *   createArtifacts: 빌드가 설치본 서명(.sig)까지 만들게 한다(개인키가 있는 로컬 빌드). 기본은 끔.
 */
export function buildUpdaterConfig({ pubkey, endpoint = DEFAULT_ENDPOINT, createArtifacts = false }) {
  const key = String(pubkey ?? '').trim()
  if (key === '') throw new Error('서명 공개키(src-tauri/updater.pub)가 비어 있습니다')
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(key)) {
    throw new Error('서명 공개키가 base64 가 아닙니다. tauri signer generate 가 만든 .pub 파일의 내용 한 줄을 그대로 두세요')
  }
  const url = new URL(endpoint)
  if (url.protocol !== 'https:' && url.protocol !== 'http:') throw new Error(`엔드포인트 주소가 올바르지 않습니다: ${endpoint}`)
  const updater = {
    pubkey: key,
    endpoints: [url.href],
    windows: { installMode: 'passive' },
    // 서명의 신뢰 주석(서명이 덮는다)에 든 버전이 피드의 version 과 같아야 받는다. 피드(latest.json)는 서명되지 않아,
    // 이것이 없으면 피드를 바꿔치는 쪽이 새 버전 번호에 옛(유효하게 서명된) 설치본을 붙여 다운그레이드시킬 수 있다.
    // 서명은 `tauri signer sign --app-version` 이나 `tauri build` 가 만들며 항상 `version:` 이 들어 있다.
    requireSignedVersion: true
  }
  if (url.protocol === 'http:') updater.dangerousInsecureTransportProtocol = true
  const config = { plugins: { updater } }
  if (createArtifacts) config.bundle = { createUpdaterArtifacts: true }
  return config
}
