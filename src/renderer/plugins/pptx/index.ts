import JSZip from 'jszip'
import type { PreviewPlugin, RenderContext } from '../types'
import { readBinary } from '../read-binary'

const NS_A = 'http://schemas.openxmlformats.org/drawingml/2006/main'
const NS_R = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
const IMAGE_MIME: Record<string, string> = {
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', bmp: 'image/bmp', webp: 'image/webp',
}

function parseXml(text: string): Document {
  const doc = new DOMParser().parseFromString(text, 'application/xml')
  if (doc.getElementsByTagName('parsererror').length) throw new Error('PPTX XML 을 해석할 수 없습니다.')
  return doc
}

/** slideN.xml 의 a:p 단위 텍스트(비어 있는 문단 제외) */
function paragraphs(doc: Document): string[] {
  const out: string[] = []
  for (const p of Array.from(doc.getElementsByTagNameNS(NS_A, 'p'))) {
    const text = Array.from(p.getElementsByTagNameNS(NS_A, 't'))
      .map((t) => t.textContent ?? '')
      .join('')
    if (text.trim()) out.push(text)
  }
  return out
}

export function create(): PreviewPlugin {
  const urls: string[] = []
  return {
    async render(ctx: RenderContext) {
      const { container, signal, setSubtitle } = ctx
      const zip = await JSZip.loadAsync(await readBinary(ctx))
      if (signal.aborted) return

      const slidePaths = Object.keys(zip.files)
        .map((p) => ({ p, m: /^ppt\/slides\/slide(\d+)\.xml$/.exec(p) }))
        .filter((x): x is { p: string; m: RegExpExecArray } => !!x.m)
        .sort((a, b) => Number(a.m[1]) - Number(b.m[1]))
        .map((x) => x.p)
      if (!slidePaths.length) throw new Error('슬라이드를 찾을 수 없습니다.')

      container.classList.add('pptx-view')
      const notice = document.createElement('div')
      notice.className = 'notice'
      notice.textContent = '간이 미리보기(텍스트·이미지만)'
      const list = document.createElement('div')
      list.className = 'pptx-list'
      container.append(notice, list)

      for (let i = 0; i < slidePaths.length; i++) {
        const path = slidePaths[i]
        const doc = parseXml(await zip.file(path)!.async('string'))
        if (signal.aborted) return

        const card = document.createElement('section')
        card.className = 'pptx-card'
        const no = document.createElement('div')
        no.className = 'pptx-no'
        no.textContent = String(i + 1)
        card.append(no)

        const body = document.createElement('div')
        body.className = 'pptx-body'
        const imgUrl = await firstImage(zip, path, doc)
        if (signal.aborted) {
          if (imgUrl) URL.revokeObjectURL(imgUrl)
          return
        }
        if (imgUrl) {
          urls.push(imgUrl)
          const img = document.createElement('img')
          img.className = 'pptx-thumb'
          img.src = imgUrl
          img.alt = ''
          img.draggable = false
          body.append(img)
        }
        const paras = paragraphs(doc)
        const text = document.createElement('div')
        text.className = 'pptx-text'
        if (paras.length) {
          paras.forEach((t, idx) => {
            const p = document.createElement('p')
            if (idx === 0) p.className = 'pptx-title'
            p.textContent = t
            text.append(p)
          })
        } else {
          text.classList.add('pptx-empty')
          text.textContent = '(텍스트 없음)'
        }
        body.append(text)
        card.append(body)
        list.append(card)
      }

      setSubtitle(`PPTX · ${slidePaths.length}슬라이드`)
    },
    dispose() {
      urls.forEach((u) => URL.revokeObjectURL(u))
      urls.length = 0
    },
  }
}

/** 슬라이드가 참조하는 첫 이미지를 blob URL 로. 없거나 읽을 수 없으면 undefined. */
async function firstImage(zip: JSZip, slidePath: string, slide: Document): Promise<string | undefined> {
  const dir = slidePath.slice(0, slidePath.lastIndexOf('/'))
  const relsFile = zip.file(`${dir}/_rels/${slidePath.slice(dir.length + 1)}.rels`)
  if (!relsFile) return undefined
  const rels = parseXml(await relsFile.async('string'))
  const targets = new Map<string, string>()
  for (const r of Array.from(rels.getElementsByTagName('Relationship'))) {
    if (/\/image$/.test(r.getAttribute('Type') ?? '')) targets.set(r.getAttribute('Id') ?? '', r.getAttribute('Target') ?? '')
  }
  // 슬라이드 본문에서 처음 등장하는 r:embed 순서를 따른다.
  for (const blip of Array.from(slide.getElementsByTagNameNS(NS_A, 'blip'))) {
    const target = targets.get(blip.getAttributeNS(NS_R, 'embed') ?? '')
    if (!target) continue
    const path = resolvePath(dir, target)
    const file = zip.file(path)
    const mime = IMAGE_MIME[path.split('.').pop()?.toLowerCase() ?? '']
    if (!file || !mime) continue
    const bytes = await file.async('uint8array')
    return URL.createObjectURL(new Blob([bytes as Uint8Array<ArrayBuffer>], { type: mime }))
  }
  return undefined
}

/** 관계 Target(상대 경로 또는 /절대 경로)을 zip 내부 경로로 */
function resolvePath(dir: string, target: string): string {
  if (target.startsWith('/')) return target.slice(1)
  const parts = dir.split('/')
  for (const seg of target.split('/')) {
    if (seg === '..') parts.pop()
    else if (seg && seg !== '.') parts.push(seg)
  }
  return parts.join('/')
}
