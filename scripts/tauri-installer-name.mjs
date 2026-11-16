// `tauri build --bundles nsis` 가 만든 설치본을 Electron 설치본과 같은 이름(Peekaboo-Setup-<버전>-x64.exe)으로
// release-tauri/ 에 복사한다. Tauri 의 산출물 이름(Peekaboo_<버전>_x64-setup.exe)은 바꿀 수 없어 빌드 뒤에 옮긴다.
// 서명 파일(.sig)이 있으면 같은 이름으로 함께 옮긴다.
// 타깃 폴더는 CARGO_TARGET_DIR 을 따른다(상대 경로는 cargo 를 돌리는 src-tauri 기준).
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const tauriDir = join(root, 'src-tauri')
const targetDir = process.env.CARGO_TARGET_DIR ? resolve(tauriDir, process.env.CARGO_TARGET_DIR) : join(tauriDir, 'target')
const nsisDir = join(targetDir, 'release', 'bundle', 'nsis')
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version

const candidates = readdirSync(nsisDir)
  .filter((name) => name.endsWith('-setup.exe'))
  .map((name) => ({ name, mtime: statSync(join(nsisDir, name)).mtimeMs }))
  .sort((a, b) => b.mtime - a.mtime)
if (candidates.length === 0) {
  console.error(`설치본을 찾지 못했습니다: ${nsisDir}`)
  process.exit(1)
}

const outDir = join(root, 'release-tauri')
mkdirSync(outDir, { recursive: true })
const out = join(outDir, `Peekaboo-Setup-${version}-x64.exe`)
copyFileSync(join(nsisDir, candidates[0].name), out)
console.log(`${out} (${statSync(out).size} bytes)`)

// 서명 키로 빌드했으면 설치본 옆에 서명 파일(.sig)이 있다. 같은 이름으로 옮겨 update-feeds.mjs 가 찾게 한다.
// 서명은 설치본을 만든 뒤에 생기므로, 설치본보다 오래된 .sig 는 이전 서명 빌드의 잔재다(서명 없이 다시 빌드했을 때 짝이 맞지 않는 서명을 싣지 않는다).
const installer = join(nsisDir, candidates[0].name)
const sig = `${installer}.sig`
if (existsSync(sig) && statSync(sig).mtimeMs >= statSync(installer).mtimeMs) {
  copyFileSync(sig, `${out}.sig`)
  console.log(`${out}.sig`)
} else {
  rmSync(`${out}.sig`, { force: true })
}
