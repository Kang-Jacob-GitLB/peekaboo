// updater-config.mjs 의 타입(시험이 .ts 에서 가져다 쓴다)
export const DEFAULT_ENDPOINT: string
export const PUBKEY_FILE: string
export function readPubkey(file?: string): string | null
export function buildUpdaterConfig(options: { pubkey: string; endpoint?: string; createArtifacts?: boolean }): {
  bundle?: { createUpdaterArtifacts: boolean }
  plugins: {
    updater: {
      pubkey: string
      endpoints: string[]
      windows: { installMode: string }
      requireSignedVersion: boolean
      dangerousInsecureTransportProtocol?: boolean
    }
  }
}
