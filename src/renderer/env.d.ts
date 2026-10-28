import type { PeekabooApi } from '../shared/preview-api'

declare global {
  interface Window {
    peekaboo: PeekabooApi
  }
}

export {}
