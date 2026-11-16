// `tauri build`(NSIS) 실행기. npm run dist:tauri 가 부른다.
// 업데이터 설정(공개키·엔드포인트·requireSignedVersion)은 저장소의 src-tauri/updater.pub 가 있을 때만 얹는다
// (scripts/updater-config.mjs 가 만든 release-tauri/updater.conf.json 을 `tauri build --config` 로). 파일이 없으면 업데이터 없는 빌드다.
// 설치본 서명(.sig)은 개인키가 있는 로컬 빌드에서만 이 빌드가 함께 만든다(`createUpdaterArtifacts`).
// CI(`CI` 환경 변수)는 서명하지 않는다: 빌드 잡은 개인키를 받지 않고, 별도 sign 잡이 설치본에 서명한다(.github/workflows/tauri-build.yml).
//   TAURI_SIGNING_PRIVATE_KEY           (로컬) 개인키 파일 경로 또는 내용
//   TAURI_SIGNING_PRIVATE_KEY_PASSWORD  (로컬) 개인키 비밀번호(없으면 빈 값으로 둬 CLI 가 입력을 기다리며 멈추지 않게 한다)
//   PEEKABOO_UPDATER_ENDPOINT           (선택) 시험용 피드 주소. 로컬 업데이트 리허설용(docs/tauri-rehearsal.md). CI 에서는 거부한다.
import { spawnSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { DEFAULT_ENDPOINT, buildUpdaterConfig, readPubkey } from './updater-config.mjs'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const env = { ...process.env }
const args = ['build', '--bundles', 'nsis', '--config', 'src-tauri/tauri.bundle.conf.json']

const ci = (env.CI ?? '') !== ''
const pubkey = readPubkey()
const endpoint = env.PEEKABOO_UPDATER_ENDPOINT || undefined
if (ci && endpoint !== undefined && endpoint !== DEFAULT_ENDPOINT) {
  // 릴리스 후보가 시험용 피드를 보게 박히는 것을 막는다(시험용 주소는 로컬 빌드에서만)
  console.error(`CI 에서는 기본 엔드포인트(${DEFAULT_ENDPOINT}) 외의 PEEKABOO_UPDATER_ENDPOINT 를 쓸 수 없습니다: ${endpoint}`)
  process.exit(1)
}
if (pubkey !== null) {
  // CI 는 개인키를 환경에 두지 않으므로(있어도 쓰지 않는다) 서명 산출물은 만들지 않는다
  const createArtifacts = !ci && (env.TAURI_SIGNING_PRIVATE_KEY ?? '') !== ''
  let config
  try {
    config = buildUpdaterConfig({ pubkey, endpoint, createArtifacts })
  } catch (e) {
    console.error(e instanceof Error ? e.message : e)
    process.exit(1)
  }
  const outDir = join(root, 'release-tauri')
  mkdirSync(outDir, { recursive: true })
  const file = join(outDir, 'updater.conf.json')
  writeFileSync(file, `${JSON.stringify(config, null, 2)}\n`)
  args.push('--config', file)
  if (createArtifacts) env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= ''
  else delete env.TAURI_SIGNING_PRIVATE_KEY
  console.log(`업데이터 설정을 얹습니다(공개키: src-tauri/updater.pub, 설치본 서명 ${createArtifacts ? '함' : '안 함'})`)
} else {
  // 개인키가 환경에 있어도 쓰지 않는다. 서명 산출물 없이 나간 빌드가 서명된 것처럼 보이지 않게 개인키는 자식 프로세스에도 넘기지 않는다.
  delete env.TAURI_SIGNING_PRIVATE_KEY
  console.log('src-tauri/updater.pub 가 없어 업데이터 없이 빌드합니다(docs/tauri-migration.md 6절: 키 생성·커밋 절차)')
}
args.push('--', '--locked')

// .bin 셔임(tauri.cmd)은 셸이 필요하므로 CLI 의 JS 진입점을 node 로 직접 실행한다
const cli = join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
const result = spawnSync(process.execPath, [cli, ...args], { cwd: root, env, stdio: 'inherit' })
process.exit(result.status ?? 1)
