// next-version.mjs 의 타입(시험이 .ts 에서 가져다 쓴다)
export interface Commit {
  hash: string
  subject: string
  body: string
}
export type Bump = 'patch' | 'minor' | 'major'
export const RELEASE_NOTES_TAIL: string
export function classify(
  subject: string,
  body?: string
): { type: string; scope: string | null; breaking: boolean; bump: Bump | null; title: string }
export function decideBump(commits: Pick<Commit, 'subject' | 'body'>[]): Bump | null
export function bumpVersion(lastTag: string, bump: Bump): string
export function releaseNotes(next: string, commits: Pick<Commit, 'subject' | 'body'>[]): string
