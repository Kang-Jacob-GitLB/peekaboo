import hljs from 'highlight.js/lib/core'

// Vite 가 언어별로 청크를 나눌 수 있도록 정적 import() 맵을 쓴다.
type Loader = () => Promise<{ default: Parameters<typeof hljs.registerLanguage>[1] }>
const loaders: Record<string, Loader> = {
  bash: () => import('highlight.js/lib/languages/bash'),
  c: () => import('highlight.js/lib/languages/c'),
  cpp: () => import('highlight.js/lib/languages/cpp'),
  csharp: () => import('highlight.js/lib/languages/csharp'),
  css: () => import('highlight.js/lib/languages/css'),
  dart: () => import('highlight.js/lib/languages/dart'),
  diff: () => import('highlight.js/lib/languages/diff'),
  dockerfile: () => import('highlight.js/lib/languages/dockerfile'),
  dos: () => import('highlight.js/lib/languages/dos'),
  go: () => import('highlight.js/lib/languages/go'),
  gradle: () => import('highlight.js/lib/languages/gradle'),
  ini: () => import('highlight.js/lib/languages/ini'),
  java: () => import('highlight.js/lib/languages/java'),
  javascript: () => import('highlight.js/lib/languages/javascript'),
  json: () => import('highlight.js/lib/languages/json'),
  kotlin: () => import('highlight.js/lib/languages/kotlin'),
  lua: () => import('highlight.js/lib/languages/lua'),
  makefile: () => import('highlight.js/lib/languages/makefile'),
  markdown: () => import('highlight.js/lib/languages/markdown'),
  php: () => import('highlight.js/lib/languages/php'),
  powershell: () => import('highlight.js/lib/languages/powershell'),
  properties: () => import('highlight.js/lib/languages/properties'),
  python: () => import('highlight.js/lib/languages/python'),
  r: () => import('highlight.js/lib/languages/r'),
  ruby: () => import('highlight.js/lib/languages/ruby'),
  rust: () => import('highlight.js/lib/languages/rust'),
  scss: () => import('highlight.js/lib/languages/scss'),
  sql: () => import('highlight.js/lib/languages/sql'),
  swift: () => import('highlight.js/lib/languages/swift'),
  typescript: () => import('highlight.js/lib/languages/typescript'),
  xml: () => import('highlight.js/lib/languages/xml'),
  yaml: () => import('highlight.js/lib/languages/yaml'),
}

// 확장자 / 파일명 / 코드펜스 태그 → 언어 모듈 이름
const aliases: Record<string, string> = {
  js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'javascript',
  ts: 'typescript', tsx: 'typescript',
  html: 'xml', htm: 'xml', svg: 'xml', vue: 'xml', svelte: 'xml',
  yml: 'yaml', toml: 'ini', cfg: 'ini', conf: 'ini', env: 'ini', '.env': 'ini',
  editorconfig: 'ini', '.editorconfig': 'ini',
  py: 'python', rb: 'ruby', rs: 'rust', kt: 'kotlin', h: 'c', hpp: 'cpp', cc: 'cpp', 'c++': 'cpp',
  cs: 'csharp', sh: 'bash', shell: 'bash', zsh: 'bash', ps1: 'powershell', bat: 'dos', cmd: 'dos',
  patch: 'diff', md: 'markdown', mdx: 'markdown', jsonc: 'json', make: 'makefile', docker: 'dockerfile',
}

export function resolveLanguage(key: string): string | undefined {
  const k = key.toLowerCase()
  const name = aliases[k] ?? k
  return Object.hasOwn(loaders, name) ? name : undefined
}

const pending = new Map<string, Promise<void>>()

/** 언어 모듈을 (한 번만) 로드해 등록하고 정규화된 언어명을 돌려준다. 알 수 없으면 undefined. */
export async function ensureLanguage(key: string): Promise<string | undefined> {
  const name = resolveLanguage(key)
  if (!name) return undefined
  if (!hljs.getLanguage(name)) {
    let p = pending.get(name)
    if (!p) {
      p = loaders[name]().then((m) => hljs.registerLanguage(name, m.default))
      pending.set(name, p)
    }
    await p
  }
  return name
}

export function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

/** ensureLanguage 가 끝난 언어에 대해 동기 하이라이트. 실패하면 이스케이프된 평문. */
export function highlightSync(code: string, language: string | undefined): string {
  if (language && hljs.getLanguage(language)) {
    try {
      return hljs.highlight(code, { language, ignoreIllegals: true }).value
    } catch {
      /* fallthrough */
    }
  }
  return escapeHtml(code)
}
