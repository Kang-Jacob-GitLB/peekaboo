import { readdirSync, readFileSync, existsSync } from 'node:fs'
import { resolve, join, extname } from 'node:path'
import { defineConfig } from 'electron-vite'
import type { Plugin } from 'vite'

// pdf.js 가 런타임에 가져오는 CMap·표준 폰트·wasm 디렉터리. 외부 CDN 대신 번들에 같이 싣는다.
const PDFJS_DIRS = ['cmaps', 'standard_fonts', 'wasm']
const pdfjsRoot = resolve(__dirname, 'node_modules/pdfjs-dist')

const MIME: Record<string, string> = { '.wasm': 'application/wasm', '.js': 'text/javascript' }

/** 렌더러 출력(build)과 dev 서버 양쪽의 /pdfjs/<dir>/<file> 에 pdfjs-dist 자산을 노출한다. */
function pdfjsAssets(): Plugin {
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

export default defineConfig({
  main: {
    build: { rollupOptions: { input: { index: resolve(__dirname, 'src/main/index.ts') } } }
  },
  preload: {
    build: { rollupOptions: { input: { index: resolve(__dirname, 'src/preload/index.ts') } } }
  },
  renderer: {
    root: resolve(__dirname, 'src/renderer'),
    plugins: [pdfjsAssets()],
    build: { rollupOptions: { input: { index: resolve(__dirname, 'src/renderer/index.html') } } }
  }
})
