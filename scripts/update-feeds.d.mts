// update-feeds.mjs 의 타입(시험이 .ts 에서 가져다 쓴다)
export const REPO: string
export const PLATFORM: string
export function installerName(version: string): string
export function installerUrl(version: string, repo?: string): string
export function normalizeSignature(raw: unknown): string
export interface Feed {
  version: string
  notes: string
  pub_date: string
  platforms: Record<string, { signature: string; url: string }>
}
export function buildFeed(options: {
  version: string
  notes?: string
  signature: string
  pubDate?: Date | string
  repo?: string
  baseUrl?: string
}): Feed
