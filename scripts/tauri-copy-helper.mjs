// 헬퍼 산출물(helper/publish/peekaboo-helper.exe)을 Tauri 의 externalBin 이름으로 복사한다.
// `bundle.externalBin: ["binaries/peekaboo-helper"]` 는 `binaries/peekaboo-helper-<타깃 트리플>.exe` 를 찾는다.
// 사용: npm run helper:build 뒤에 `node scripts/tauri-copy-helper.mjs` (npm run dist:tauri 가 이어서 한다)
import { copyFileSync, existsSync, mkdirSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const source = join(root, 'helper', 'publish', 'peekaboo-helper.exe')
// 지원 타깃은 x64 하나다(README "Windows 10 1809 이상 / Windows 11 (x64)")
const target = join(root, 'src-tauri', 'binaries', 'peekaboo-helper-x86_64-pc-windows-msvc.exe')

if (!existsSync(source)) {
  console.error(`헬퍼가 없습니다: ${source}\n먼저 \`npm run helper:build\` 를 실행하세요.`)
  process.exit(1)
}
mkdirSync(dirname(target), { recursive: true })
copyFileSync(source, target)
console.log(`${target} (${statSync(target).size} bytes)`)
