// updater-sig.mjs 의 타입(시험이 .ts 에서 가져다 쓴다)
export function parsePublicKey(pubkey: string): { keyId: Buffer; key: Buffer }
export function parseSignature(signature: string): {
  algorithm: string
  keyId: Buffer
  signature: Buffer
  trustedComment: string
  globalSignature: Buffer
}
export function signedVersion(trustedComment: string): string | undefined
export function verifyUpdateSignature(input: { installer: Uint8Array; signature: string; pubkey: string; version: string }): void
