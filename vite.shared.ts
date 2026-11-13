// electron.vite.config.ts 와 vite.tauri.config.ts 가 함께 쓰는 렌더러 빌드 설정.

import { readdirSync, readFileSync, existsSync } from 'node:fs'
import { resolve, join, extname } from 'node:path'
import type { Plugin } from 'vite'

// pdf.js 가 런타임에 가져오는 CMap·표준 폰트·wasm 디렉터리. 외부 CDN 대신 번들에 같이 싣는다.
const PDFJS_DIRS = ['cmaps', 'standard_fonts', 'wasm']
const pdfjsRoot = resolve(__dirname, 'node_modules/pdfjs-dist')

const MIME: Record<string, string> = { '.wasm': 'application/wasm', '.js': 'text/javascript' }

/** 렌더러 출력(build)과 dev 서버 양쪽의 /pdfjs/<dir>/<file> 에 pdfjs-dist 자산을 노출한다. */
export function pdfjsAssets(): Plugin {
  return {
    name: 'peekaboo:pdfjs-assets',
    configureServer(server) {
      server.middlewares.use('/pdfjs', (req, res, next) => {
        const rel = decodeURIComponent((req.url ?? '').split('?')[0]).replace(/^\/+/, '')
        const [dir, name, ...rest] = rel.split('/')
        if (rest.length || !PDFJS_DIRS.includes(dir) || !name || name.includes('..')) return next()
        const file = join(pdfjsRoot, dir, name)
        if (!existsSync(file)) return next()
        res.setHeader('Content-Type', MIME[extname(file)] ?? 'application/octet-stream')
        res.end(readFileSync(file))
      })
    },
    generateBundle() {
      for (const dir of PDFJS_DIRS) {
        for (const name of readdirSync(join(pdfjsRoot, dir))) {
          this.emitFile({ type: 'asset', fileName: `pdfjs/${dir}/${name}`, source: readFileSync(join(pdfjsRoot, dir, name)) })
        }
      }
    }
  }
}

/** 빌드 대상. 렌더러 코드는 `import.meta.env.VITE_HOST` 로 어댑터를 고른다(host/index.ts). */
export type HostKind = 'electron' | 'tauri'

/** VITE_HOST 를 컴파일 타임 상수로 박아, 고르지 않은 쪽 어댑터가 번들에서 빠지게 한다. */
export function hostDefine(host: HostKind): Record<string, string> {
  return { 'import.meta.env.VITE_HOST': JSON.stringify(host) }
}

/**
 * index.html 의 CSP <meta> 를 뺀다. Tauri 는 tauri.conf.json 의 app.security.csp 를 응답 헤더로 주므로
 * (peek 출처가 http://peek.localhost 라 Electron 의 meta 와 값도 다르다) meta 가 남아 있으면 둘이 겹쳐 제한만 늘어난다.
 */
export function stripCspMeta(): Plugin {
  return {
    name: 'peekaboo:strip-csp-meta',
    transformIndexHtml: {
      order: 'pre',
      handler: (html) => html.replace(/\s*<meta\s+http-equiv="Content-Security-Policy"[^>]*>/i, '')
    }
  }
}
