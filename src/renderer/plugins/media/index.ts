import type { PreviewPlugin, RenderContext } from '../types'

const AUDIO_EXT = new Set(['mp3', 'm4a', 'aac', 'wav', 'ogg', 'oga', 'opus', 'flac'])

const NOTE_ICON = `<svg viewBox="0 0 64 64" width="112" height="112" aria-hidden="true" focusable="false">
<circle cx="32" cy="32" r="28" fill="var(--icon-body)" stroke="var(--icon-edge)" stroke-width="2"/>
<path d="M26 44V20l16-4v24" fill="none" stroke="var(--accent)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>
<circle cx="22" cy="44" r="5" fill="var(--accent)"/>
<circle cx="38" cy="40" r="5" fill="var(--accent)"/>
</svg>`

/** 3:25, 1:02:03 형태. */
function formatDuration(sec: number): string {
  const s = Math.max(0, Math.round(sec))
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const ss = String(s % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`
}

// 재생 가능 여부는 Chromium 이 결정한다(코덱 미지원이면 error 이벤트 → 폴백 카드).
export function create(): PreviewPlugin {
  let media: HTMLMediaElement | null = null

  return {
    async render({ file, container, signal, host, setSubtitle }: RenderContext) {
      const isAudio = AUDIO_EXT.has(file.ext)
      const el = document.createElement(isAudio ? 'audio' : 'video')
      media = el
      el.controls = true
      el.preload = 'metadata'
      el.loop = false
      if (el instanceof HTMLVideoElement) el.disablePictureInPicture = true

      if (isAudio) {
        container.classList.add('media-view', 'audio-view')
        const icon = document.createElement('div')
        icon.className = 'media-icon'
        icon.innerHTML = NOTE_ICON
        const name = document.createElement('div')
        name.className = 'media-name'
        name.textContent = file.name
        container.append(icon, name, el)
      } else {
        container.classList.add('media-view', 'video-view')
        container.append(el)
      }

      await new Promise<void>((resolve, reject) => {
        el.onloadedmetadata = () => resolve()
        el.onerror = () => reject(new Error('이 코덱은 지원하지 않아 재생할 수 없습니다.'))
        signal.addEventListener('abort', () => resolve(), { once: true })
        el.src = host.fileUrl(file.path)
      })
      if (signal.aborted) return

      const parts = [file.ext.toUpperCase()]
      if (el instanceof HTMLVideoElement && el.videoWidth && el.videoHeight) {
        parts.push(`${el.videoWidth}×${el.videoHeight}`)
      }
      if (Number.isFinite(el.duration)) parts.push(formatDuration(el.duration))
      setSubtitle(parts.join(' · '))

      // 창이 포커스를 받지 않아 클릭 재생이 어려우므로 자동 재생한다. 정책상 거부돼도 컨트롤로 재생할 수 있다.
      el.play().catch(() => {})
    },

    dispose() {
      if (!media) return
      // 창을 닫거나 다음 파일로 넘어가도 소리가 남지 않도록 디코더까지 해제한다.
      media.onloadedmetadata = null
      media.onerror = null
      media.pause()
      media.removeAttribute('src')
      media.load()
      media = null
    },
  }
}
