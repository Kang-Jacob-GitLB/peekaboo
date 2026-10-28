import { net, protocol } from 'electron'
import { pathToFileURL } from 'node:url'
import { PEEK_SCHEME, fromPeekUrl } from '../shared/channels'

/** app ready 이전에 호출해야 한다. stream: true 로 영상/오디오 Range 요청을 지원한다. */
export function registerPeekScheme(): void {
  protocol.registerSchemesAsPrivileged([
    { scheme: PEEK_SCHEME, privileges: { standard: false, secure: true, stream: true, supportFetchAPI: true } }
  ])
}

/**
 * peek://file/<경로> 핸들러. isAllowed 가 true 인 경로(= 현재 미리보기 중인 파일)만 서빙해서,
 * 렌더러가 임의의 로컬 파일을 읽지 못하게 한다.
 */
export function handlePeekProtocol(isAllowed: (path: string) => boolean): void {
  protocol.handle(PEEK_SCHEME, (request) => {
    const path = fromPeekUrl(request.url)
    if (!path || !isAllowed(path)) return new Response('Forbidden', { status: 403 })
    return net.fetch(pathToFileURL(path).toString(), { headers: request.headers })
  })
}
