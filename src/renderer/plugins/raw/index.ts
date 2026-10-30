import type { PreviewPlugin, RenderContext } from '../types'
import { readBinary } from '../read-binary'

// 헤더 없는 카메라 프레임 덤프(.raw/.yuv/.nv12). 파일에 해상도·포맷 정보가 없으므로
// 파일 크기 = 가로 × 세로 × 픽셀당 바이트 가 되는 조합을 후보로 만들고, 헤더의 선택 상자로 바꿔 볼 수 있게 한다.

type Format = 'nv12' | 'nv21' | 'i420' | 'yuyv' | 'uyvy' | 'rgb24' | 'bgr24' | 'rgba' | 'bgra' | 'gray'

// 같은 바이트 수끼리 묶는다. 묶음 안 순서 = 기본 선택 우선순위
const FORMAT_GROUPS: { bytesPerPixel: number; formats: Format[] }[] = [
  { bytesPerPixel: 1.5, formats: ['nv12', 'nv21', 'i420'] },
  { bytesPerPixel: 2, formats: ['yuyv', 'uyvy'] },
  { bytesPerPixel: 3, formats: ['rgb24', 'bgr24'] },
  { bytesPerPixel: 4, formats: ['bgra', 'rgba'] },
  { bytesPerPixel: 1, formats: ['gray'] },
]

// 흔한 순서. 같은 크기로 여러 해상도가 맞으면 앞쪽을 기본으로 고른다.
// 1920×1088 은 하드웨어 인코더·ISP 가 16줄 정렬로 내보내는 FHD 다.
const RESOLUTIONS: [number, number][] = [
  [1920, 1080], [1280, 720], [3840, 2160], [1920, 1088], [640, 480], [1280, 800], [1920, 1200],
  [1280, 960], [1024, 768], [800, 480], [1024, 600], [800, 600], [720, 480], [720, 576],
  [1280, 1024], [1600, 1200], [2048, 1536], [2560, 1440], [2592, 1944], [4096, 2160],
]

interface Candidate { format: Format; width: number; height: number }

function findCandidates(size: number): Candidate[] {
  const out: Candidate[] = []
  for (const [width, height] of RESOLUTIONS) {
    for (const { bytesPerPixel, formats } of FORMAT_GROUPS) {
      if (width * height * bytesPerPixel !== size) continue
      for (const format of formats) out.push({ format, width, height })
    }
  }
  return out
}

/**
 * YUV420 세 종류 중 NV12/NV21(UV 가 바이트마다 번갈아 옴)과 I420(U·V 평면이 따로)을 가른다.
 * 인터리브면 이웃 바이트(U↔V) 차이가 두 칸 건너(U↔U) 차이보다 훨씬 크다.
 */
function looksPlanar(data: Uint8Array, width: number, height: number): boolean {
  const start = width * height
  const end = Math.min(data.length - 2, start + 200_000)
  let adjacent = 0
  let stride2 = 0
  for (let i = start; i < end; i += 3) {
    adjacent += Math.abs(data[i] - data[i + 1])
    stride2 += Math.abs(data[i] - data[i + 2])
  }
  return adjacent < stride2 * 1.2
}

const clamp = (v: number) => (v < 0 ? 0 : v > 255 ? 255 : v)

/** BT.601 제한 범위(16–235) YUV → RGBA 한 픽셀 */
function putYuv(out: Uint8ClampedArray, o: number, y: number, u: number, v: number): void {
  const c = (y - 16) * 298
  const d = u - 128
  const e = v - 128
  out[o] = clamp((c + 409 * e + 128) >> 8)
  out[o + 1] = clamp((c - 100 * d - 208 * e + 128) >> 8)
  out[o + 2] = clamp((c + 516 * d + 128) >> 8)
  out[o + 3] = 255
}

function decode(data: Uint8Array, { format, width: w, height: h }: Candidate): ImageData {
  const img = new ImageData(w, h)
  const out = img.data
  const n = w * h
  switch (format) {
    case 'nv12':
    case 'nv21': {
      const [uOff, vOff] = format === 'nv12' ? [0, 1] : [1, 0]
      for (let y = 0; y < h; y++) {
        const uvRow = n + (y >> 1) * w
        for (let x = 0; x < w; x++) {
          const uv = uvRow + (x & ~1)
          putYuv(out, (y * w + x) * 4, data[y * w + x], data[uv + uOff], data[uv + vOff])
        }
      }
      break
    }
    case 'i420': {
      const cw = w >> 1
      const vPlane = n + (n >> 2)
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const ci = (y >> 1) * cw + (x >> 1)
          putYuv(out, (y * w + x) * 4, data[y * w + x], data[n + ci], data[vPlane + ci])
        }
      }
      break
    }
    case 'yuyv':
    case 'uyvy': {
      // 두 픽셀이 4바이트(Y0 U Y1 V / U Y0 V Y1)를 나눠 쓴다
      const [y0, u, y1, v] = format === 'yuyv' ? [0, 1, 2, 3] : [1, 0, 3, 2]
      for (let i = 0, p = 0; p < n; i += 4, p += 2) {
        putYuv(out, p * 4, data[i + y0], data[i + u], data[i + v])
        putYuv(out, p * 4 + 4, data[i + y1], data[i + u], data[i + v])
      }
      break
    }
    case 'rgb24':
    case 'bgr24':
    case 'rgba':
    case 'bgra': {
      const step = format === 'rgb24' || format === 'bgr24' ? 3 : 4
      const [r, b] = format.startsWith('rgb') ? [0, 2] : [2, 0]
      for (let p = 0, i = 0; p < n; p++, i += step) {
        out[p * 4] = data[i + r]
        out[p * 4 + 1] = data[i + 1]
        out[p * 4 + 2] = data[i + b]
        // 알파 채널은 덤프마다 0 으로 비어 있는 경우가 많아 무시한다
        out[p * 4 + 3] = 255
      }
      break
    }
    case 'gray':
      for (let p = 0; p < n; p++) {
        out[p * 4] = out[p * 4 + 1] = out[p * 4 + 2] = data[p]
        out[p * 4 + 3] = 255
      }
      break
  }
  return img
}

const label = (c: Candidate) => `${c.format.toUpperCase()} · ${c.width}×${c.height}`

export function create(): PreviewPlugin {
  // 크기가 어떤 프레임과도 맞지 않으면 폴백 카드(또는 텍스트)로 넘긴다
  let delegate: PreviewPlugin | undefined
  return {
    async render(ctx: RenderContext) {
      const { file, container, signal, setSubtitle, fitContent } = ctx
      const candidates = findCandidates(file.size)
      if (candidates.length === 0) {
        const fallback = (await import('../fallback')).create()
        if (signal.aborted) return
        delegate = fallback
        return fallback.render(ctx)
      }

      const data = new Uint8Array(await readBinary(ctx))
      if (signal.aborted) return

      // 첫 해상도의 YUV420 이면 크로마 배치를 보고 NV12/I420 중 기본값을 고른다
      let current = candidates[0]
      if (current.format === 'nv12' && looksPlanar(data, current.width, current.height)) {
        current = candidates.find((c) => c.format === 'i420' && c.width === current.width) ?? current
      }

      container.classList.add('image-view', 'raw-view')
      const canvas = document.createElement('canvas')
      const select = document.createElement('select')
      select.className = 'raw-format'
      select.title = '헤더가 없는 파일이라 크기로 추정했습니다. 다르게 보이면 형식을 바꿔 보세요.'
      candidates.forEach((c, i) => select.add(new Option(label(c), String(i), false, c === current)))
      container.append(canvas, select)

      const show = (c: Candidate) => {
        canvas.width = c.width
        canvas.height = c.height
        canvas.style.maxWidth = `min(100%, ${c.width}px)`
        canvas.style.maxHeight = `min(100%, ${c.height}px)`
        canvas.getContext('2d')!.putImageData(decode(data, c), 0, 0)
        setSubtitle(`RAW · ${label(c)}`)
        // .image-view 의 padding 12px 를 더한다
        fitContent({ width: c.width + 24, height: c.height + 24 })
      }
      show(current)
      select.addEventListener('change', () => show(candidates[Number(select.value)]))
      // 후보가 하나뿐이면 고를 것이 없다
      if (candidates.length === 1) select.hidden = true
    },
    dispose() {
      delegate?.dispose?.()
    },
  }
}
