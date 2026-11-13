import { net, protocol, session } from 'electron'
import { open, type FileHandle } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { PEEK_SCHEME, fromPeekUrl } from '../shared/channels'
import { partialHeaders, rangeBody, unsatisfiableHeaders, withPeekHeaders } from './peek-response'
import { parseRange } from './range'

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
 *
 * Range 는 여기서 직접 처리한다. net.fetch(file:) 은 Range 본문만 잘라 주면서도 상태를 200 으로 두고
 * Content-Range 를 붙이지 않으며, 범위를 벗어나면 요청 자체가 실패한다. 렌더러의 압축·텍스트 해석(host/file-ops)은
 * 206 + Content-Range 로 파일 전체 크기를 알아야 한다. 파일은 한 번만 열어 그 핸들로 크기와 본문을 얻는다.
 */
export function handlePeekProtocol(isAllowed: (path: string) => boolean): void {
  // 206 응답의 Content-Type 용. 허용되는 파일이 하나뿐이라 마지막 경로만 기억하면 된다
  let typeCache: { path: string; type: string | null } | null = null
  const contentTypeOf = async (path: string, fileUrl: string): Promise<string | null> => {
    if (typeCache?.path === path) return typeCache.type
    // 확장자별 MIME 은 net.fetch(file:) 이 정한다. 1바이트만 요청해 헤더만 얻는다
    const probe = await net.fetch(fileUrl, { headers: { Range: 'bytes=0-0' } })
    const type = probe.headers.get('content-type')
    await probe.body?.cancel()
    typeCache = { path, type }
    return type
  }

  protocol.handle(PEEK_SCHEME, async (request) => {
    const path = fromPeekUrl(request.url)
    if (!path || !isAllowed(path)) return new Response('Forbidden', { status: 403 })
    const fileUrl = pathToFileURL(path).toString()

    const rangeHeader = request.headers.get('range')
    if (rangeHeader) {
      let handle: FileHandle | null = null
      try {
        handle = await open(path, 'r')
      } catch {
        // 없는 파일 등: 아래 일반 경로에서 net.fetch 가 실패하는 기존 동작을 따른다
      }
      if (handle) {
        let owned = true // 본문 스트림에 넘기기 전까지는 여기서 닫는다
        try {
          const size = (await handle.stat()).size
          const range = parseRange(rangeHeader, size)
          if (range.kind === 'unsatisfiable') return new Response(null, { status: 416, headers: unsatisfiableHeaders(size) })
          if (range.kind === 'ok') {
            const type = await contentTypeOf(path, fileUrl)
            const body = rangeBody(handle, range.start, range.end)
            owned = false
            return new Response(body, { status: 206, statusText: 'Partial Content', headers: partialHeaders(range.start, range.end, size, type) })
          }
        } finally {
          if (owned) await handle.close().catch(() => {})
        }
      }
    }

    // 해석하지 못한 Range(다중 범위 등)는 무시하고 전체를 준다. 그대로 넘기면 net.fetch 가 잘린 본문을 200 으로 준다.
    const plain = new Headers(request.headers)
    plain.delete('range')
    const res = await net.fetch(fileUrl, { headers: plain })
    return new Response(res.body, { status: res.status, statusText: res.statusText, headers: withPeekHeaders(res.headers) })
  })
}

/**
 * 호스트가 붙은 file:// 요청(= \서버\공유 UNC 경로)을 막는다. 미리보기 중인 HTML·Markdown 이
 * <img src="file://서버/…"> 같은 것으로 SMB 접속(NTLM 해시 전송)을 일으키지 못하게 하기 위함이다.
 * 앱 CSP 의 'self' 는 file: 출처에서 호스트를 가리지 않아 이것만으로는 막히지 않는다.
 * 지금 미리보기 중인 파일 자체가 네트워크 공유에 있는 경우는 허용한다.
 */
export function blockRemoteFileRequests(isAllowed: (path: string) => boolean): void {
  session.defaultSession.webRequest.onBeforeRequest((details, callback) => {
    let cancel = false
    if (details.url.startsWith('file:')) {
      try {
        const url = new URL(details.url)
        cancel = url.hostname !== '' && !isAllowed(fileURLToPath(url))
      } catch {
        cancel = true
      }
    }
    callback({ cancel })
  })
}
