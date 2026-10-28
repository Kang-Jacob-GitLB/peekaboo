import { net, protocol } from 'electron'
import { pathToFileURL } from 'node:url'
import { PEEK_SCHEME, fromPeekUrl } from '../shared/channels'

/**
 * app ready 이전에 호출해야 한다. stream: true 로 영상/오디오 Range 요청을 지원한다.
 * corsEnabled: 렌더러(file://)가 fetch() 로 파일 바이트를 읽을 수 있게 한다(PDF/오피스 플러그인).
 */
export function registerPeekScheme(): void {
  protocol.registerSchemesAsPrivileged([
    { scheme: PEEK_SCHEME, privileges: { standard: false, secure: true, stream: true, supportFetchAPI: true, corsEnabled: true } }
  ])
}

/**
 * peek://file/<경로> 핸들러. isAllowed 가 true 인 경로(= 현재 미리보기 중인 파일)만 서빙해서,
 * 렌더러가 임의의 로컬 파일을 읽지 못하게 한다.
 */
export function handlePeekProtocol(isAllowed: (path: string) => boolean): void {
  protocol.handle(PEEK_SCHEME, async (request) => {
    const path = fromPeekUrl(request.url)
    if (!path || !isAllowed(path)) return new Response('Forbidden', { status: 403 })
    const res = await net.fetch(pathToFileURL(path).toString(), { headers: request.headers })
    // 허용된 단일 파일만 서빙하므로 모든 출처에 열어 둬도 노출 범위는 같다.
    const headers = new Headers(res.headers)
    headers.set('Access-Control-Allow-Origin', '*')
    return new Response(res.body, { status: res.status, statusText: res.statusText, headers })
  })
}
