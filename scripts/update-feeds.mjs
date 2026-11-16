#!/usr/bin/env node
// Tauri 업데이터 피드(latest.json)를 만든다. `npm run dist:tauri` 를 서명 키와 함께 돌려 `release-tauri/` 에 나온
// 설치본(Peekaboo-Setup-<버전>-x64.exe)과 그 서명(.exe.sig)이 입력이다.
//   {version, notes, pub_date, platforms: {"windows-x86_64": {signature, url}}}
// url 은 GitHub 릴리스 자산 주소다. 서명(.sig 파일의 내용 그대로)이 비어 있으면 업데이터가 설치본을 거부하므로 여기서 실패한다.
// 피드를 쓰기 전에 서명을 실제로 검증한다(scripts/updater-sig.mjs): 공개키(src-tauri/updater.pub)와 키 ID 가 같은지, 설치본에 대한
// Ed25519 서명이 맞는지, 서명이 덮는 `version:` 이 릴리스 버전과 같은지. 하나라도 어긋나면 피드를 만들지 않고 실패한다.
// 사용: node scripts/update-feeds.mjs [--dir release-tauri] [--version X.Y.Z] [--notes "<텍스트>"] [--out <파일>] [--base-url <주소>]
//   버전 기본값은 package.json(릴리스 워크플로가 태그에서 넣는다). 노트 기본값은 "Peekaboo <버전>".
//   --base-url 은 로컬 업데이트 리허설용이다(설치본을 `<주소>/<설치본 이름>` 에서 받게 한다. docs/tauri-rehearsal.md).
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { readPubkey } from './updater-config.mjs'
import { verifyUpdateSignature } from './updater-sig.mjs'

export const REPO = 'Kang-Jacob-GitLB/peekaboo'
/** Tauri 업데이터가 이 앱(Windows x64)을 찾는 키 */
export const PLATFORM = 'windows-x86_64'
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/

export function installerName(version) {
  return `Peekaboo-Setup-${version}-x64.exe`
}

export function installerUrl(version, repo = REPO) {
  return `https://github.com/${repo}/releases/download/v${version}/${installerName(version)}`
}

/** minisign 서명 파일(.sig)은 base64 한 덩어리다. 공백·개행만 걷어 내고 비었거나 base64 가 아니면 거부한다. */
export function normalizeSignature(raw) {
  const signature = String(raw ?? '').replace(/\s+/g, '')
  if (signature === '') throw new Error('서명이 비어 있습니다')
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(signature)) throw new Error('서명이 base64 가 아닙니다')
  return signature
}

/** latest.json 내용(객체). pubDate 는 Date 또는 RFC 3339 문자열. baseUrl 이 있으면 설치본 주소를 `<baseUrl>/<설치본 이름>` 으로 한다. */
export function buildFeed({ version, notes, signature, pubDate = new Date(), repo = REPO, baseUrl }) {
  if (!SEMVER.test(version ?? '')) throw new Error(`버전이 올바르지 않습니다: ${version}`)
  const date = pubDate instanceof Date ? pubDate.toISOString() : new Date(pubDate).toISOString()
  return {
    version,
    notes: notes && notes.trim() !== '' ? notes : `Peekaboo ${version}`,
    pub_date: date,
    platforms: {
      [PLATFORM]: {
        signature: normalizeSignature(signature),
        url: baseUrl ? `${baseUrl.replace(/\/+$/, '')}/${installerName(version)}` : installerUrl(version, repo)
      }
    }
  }
}

function parseArgs(argv) {
  const out = {}
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i]
    if (!key.startsWith('--')) throw new Error(`알 수 없는 인자: ${key}`)
    const value = argv[++i]
    if (value === undefined) throw new Error(`${key} 에 값이 없습니다`)
    out[key.slice(2)] = value
  }
  return out
}

function main() {
  const root = join(dirname(fileURLToPath(import.meta.url)), '..')
  const args = parseArgs(process.argv.slice(2))
  const version = args.version ?? JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version
  const dir = resolve(root, args.dir ?? 'release-tauri')
  const sigPath = join(dir, `${installerName(version)}.sig`)
  if (!existsSync(sigPath)) {
    throw new Error(`서명 파일이 없습니다: ${sigPath}\n서명 잡(tauri signer sign)이나 개인키가 있는 로컬 빌드(npm run dist:tauri)가 만든 .sig 가 필요합니다.`)
  }
  const installerPath = join(dir, installerName(version))
  if (!existsSync(installerPath)) throw new Error(`설치본이 없습니다: ${installerPath}`)
  const pubkey = readPubkey()
  if (pubkey === null) throw new Error('src-tauri/updater.pub 가 없어 서명을 검증할 수 없습니다(docs/tauri-migration.md 6절: 키 생성·커밋 절차)')
  const signature = readFileSync(sigPath, 'utf8')
  verifyUpdateSignature({ installer: readFileSync(installerPath), signature, pubkey, version })
  const feed = buildFeed({ version, notes: args.notes, signature, baseUrl: args['base-url'] })
  const out = resolve(root, args.out ?? join(dir, 'latest.json'))
  writeFileSync(out, `${JSON.stringify(feed, null, 2)}\n`)
  console.log(`${out} (v${version})`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (e) {
    console.error(e instanceof Error ? e.message : e)
    process.exit(1)
  }
}
