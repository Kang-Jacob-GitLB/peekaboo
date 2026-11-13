// `peek` 커스텀 프로토콜 (docs/tauri-migration.md 3.2절).
//
// Electron 구현(src/main/protocol.ts, preview-window.ts 의 isAllowedPath, shared/channels.ts 의
// fromPeekUrl)의 의미를 옮긴 것이다. 순수 로직(parse_range, path_from_url, build_response)과
// Tauri 결선(handle)을 나눠 두었고, 결선 이외는 전부 cargo test 로 검증한다.
// 허용 경로는 preview.rs 의 PreviewState 가 들고 있다(표시 순번과 함께 바뀌도록).
//
// wry 는 응답 본문을 통째로 메모리에 넘기므로(스트리밍 불가) Range 응답을 최대 4MiB 로 자른다.
// Range 없는 전체 응답은 최대 128MiB 를 Vec 에 올리므로 동시에 2개까지만 읽는다(FullReadLimiter).

use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::sync::{Condvar, Mutex};

use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::preview::PreviewState;

/// Range 응답 한 번에 돌려주는 최대 바이트(4MiB)
pub const MAX_RANGE_BYTES: u64 = 4 * 1024 * 1024;
/// Range 없는 전체 응답의 상한(128MiB). 넘으면 413
pub const MAX_FULL_BYTES: u64 = 128 * 1024 * 1024;
/// Range 없는 전체 읽기의 동시 상한. 넘는 요청은 슬롯이 날 때까지 기다린다(메모리 최대 2 x 128MiB).
const MAX_CONCURRENT_FULL_READS: u32 = 2;
/// 응답마다 붙는 CSP. 응답이 문서로 열려도(예: 렌더러의 링크·iframe 이동) 불투명 출처의 sandbox 가 돼
/// 스크립트·앱 출처 접근이 막힌다. 이미지·미디어·fetch 같은 하위 리소스 응답에는 CSP 가 적용되지 않아 영향이 없다.
const RESPONSE_CSP: &str = "sandbox; default-src 'none'";

// ---------------------------------------------------------------------------
// Range 파서
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub enum RangeResult {
    /// 돌려줄 구간. `end` 는 포함(inclusive)이며 이미 파일 끝과 4MiB 상한으로 잘려 있다.
    Partial { start: u64, end: u64 },
    /// 416 대상(형식 오류, 다중 구간, 범위 밖, 빈 파일)
    NotSatisfiable,
}

/// 10진수 숫자만 허용한다(`+`·`-`·공백 불가). u64 를 넘는 값은 포화시킨다.
fn parse_num(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(s.bytes().fold(0u64, |acc, b| {
        acc.saturating_mul(10).saturating_add(u64::from(b - b'0'))
    }))
}

/// `Range` 헤더 값 하나를 해석한다. 단일 구간 `bytes=a-b`, `bytes=a-`, `bytes=-n` 만 지원한다.
/// 단위(`bytes`)는 대소문자를 가리지 않고, 전체와 `=`·`-` 주변의 공백은 무시한다.
pub fn parse_range(header: &str, file_len: u64) -> RangeResult {
    use RangeResult::{NotSatisfiable, Partial};

    let Some((unit, spec)) = header.trim().split_once('=') else {
        return NotSatisfiable;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") || spec.contains(',') {
        return NotSatisfiable;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return NotSatisfiable;
    };
    let (first, last) = (first.trim(), last.trim());
    if file_len == 0 {
        return NotSatisfiable;
    }
    let last_byte = file_len - 1;

    let (start, end) = match (first.is_empty(), last.is_empty()) {
        // -n: 마지막 n 바이트(n 이 파일보다 크면 파일 전체)
        (true, false) => match parse_num(last) {
            Some(n) if n > 0 => (file_len.saturating_sub(n), last_byte),
            _ => return NotSatisfiable,
        },
        // a-
        (false, true) => match parse_num(first) {
            Some(a) => (a, last_byte),
            None => return NotSatisfiable,
        },
        // a-b
        (false, false) => match (parse_num(first), parse_num(last)) {
            (Some(a), Some(b)) if a <= b => (a, b.min(last_byte)),
            _ => return NotSatisfiable,
        },
        (true, true) => return NotSatisfiable,
    };
    if start > last_byte {
        return NotSatisfiable;
    }
    // start + 4MiB 는 start <= file_len - 1 이라 오버플로하지 않는다.
    Partial {
        start,
        end: end.min(start + MAX_RANGE_BYTES - 1),
    }
}

// ---------------------------------------------------------------------------
// URL → 경로
// ---------------------------------------------------------------------------

// Windows 에서 JS 가 부르는 URL 은 `http://peek.localhost/<encodeURIComponent(경로)>` 이다
// (tauri 2.12.5 app.rs:2331-2333). wry 는 핸들러에 넘기기 전에 이를 `peek://localhost/…` 로 되돌린다
// (wry 0.57.2 webview2/mod.rs:1190-1196, custom_protocol_workaround.rs:30-43). 어느 쪽이 와도 읽도록
// 세 접두사를 모두 받는다. 호스트·스킴은 URL 규칙대로 대소문자를 가리지 않는다.
const URL_PREFIXES: [&str; 3] = [
    "peek://localhost/",
    "http://peek.localhost/",
    "https://peek.localhost/",
];

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `decodeURIComponent` 와 같게 퍼센트 디코딩한다. `%` 뒤가 16진 두 자리가 아니거나
/// 결과가 올바른 UTF-8 이 아니면 None 이다(`+` 는 공백으로 바꾸지 않는다).
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex_val(*bytes.get(i + 1)?)?;
            let lo = hex_val(*bytes.get(i + 2)?)?;
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 요청 URI 에서 로컬 경로를 꺼낸다. 접두사가 다르거나, 퍼센트 인코딩이 잘못됐거나,
/// 경로가 비어 있거나, 날것의 `?`(쿼리)가 있으면 None 이다.
/// 렌더러가 만드는 URL 은 `encodeURIComponent(경로)` 뿐이라 `?` 는 항상 `%3F` 로 나온다. 날것의 `?` 는
/// 우리가 만들지 않은 URL 이므로 잘라서 받아 주지 않고 거부한다(다른 요청이 같은 경로로 해석될 여지를 없앤다).
/// 프래그먼트 `#` 는 브라우저가 핸들러로 보내지 않으므로 오지 않는다. 오더라도 그 앞까지만 쓴다
/// (`%23` 은 그대로 `#` 로 디코딩된다). 그래서 이쪽은 기존 동작을 유지한다.
pub fn path_from_url(uri: &str) -> Option<String> {
    let rest = URL_PREFIXES
        .iter()
        .find_map(|p| strip_prefix_ignore_case(uri, p))?;
    let encoded = rest.split('#').next().unwrap_or("");
    if encoded.contains('?') {
        return None;
    }
    let path = percent_decode(encoded)?;
    (!path.is_empty()).then_some(path)
}

// ---------------------------------------------------------------------------
// 전체 읽기 동시 상한
// ---------------------------------------------------------------------------

/// 동시에 잡을 수 있는 슬롯 수를 제한한다(Mutex + Condvar). 슬롯은 `Slot` 이 Drop 될 때 돌아온다.
pub struct FullReadLimiter {
    free: Mutex<u32>,
    cond: Condvar,
}

pub struct Slot<'a>(&'a FullReadLimiter);

impl FullReadLimiter {
    pub const fn new(max: u32) -> Self {
        FullReadLimiter {
            free: Mutex::new(max),
            cond: Condvar::new(),
        }
    }

    /// 빈 슬롯이 생길 때까지 기다렸다가 잡는다.
    pub fn acquire(&self) -> Slot<'_> {
        // 다른 스레드가 패닉해도 카운터는 단순 값이라 그대로 쓴다
        let guard = self.free.lock().unwrap_or_else(|e| e.into_inner());
        let mut guard = self
            .cond
            .wait_while(guard, |free| *free == 0)
            .unwrap_or_else(|e| e.into_inner());
        *guard -= 1;
        Slot(self)
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.0.cond.notify_one();
    }
}

static FULL_READS: FullReadLimiter = FullReadLimiter::new(MAX_CONCURRENT_FULL_READS);

// ---------------------------------------------------------------------------
// 응답 생성
// ---------------------------------------------------------------------------

/// 확장자(소문자) → Content-Type. 모르면 application/octet-stream.
fn content_type(path: &str) -> &'static str {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let ext = match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    };
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "html" | "htm" | "xhtml" => "text/html",
        "css" => "text/css",
        "js" | "mjs" | "cjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "md" | "markdown" | "mdx" | "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// 모든 응답에 붙는 공통 헤더. 허용된 단일 파일만 서빙하므로 모든 출처에 열어 둬도 노출 범위는 같다.
fn base(status: StatusCode) -> tauri::http::response::Builder {
    Response::builder()
        .status(status)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "Content-Range, Content-Length, Accept-Ranges",
        )
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_SECURITY_POLICY, RESPONSE_CSP)
}

fn error(status: StatusCode) -> Response<Vec<u8>> {
    base(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(
            status
                .canonical_reason()
                .unwrap_or("Error")
                .as_bytes()
                .to_vec(),
        )
        .unwrap()
}

/// CORS preflight(OPTIONS). `Range: bytes=-n` 은 CORS 안전 목록 헤더가 아니라 preflight 가 올 수 있다.
fn preflight() -> Response<Vec<u8>> {
    base(StatusCode::NO_CONTENT)
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Range")
        .body(Vec::new())
        .unwrap()
}

/// 파일의 `[start, start+len)` 을 seek + read 로 읽는다.
fn read_span(file: &mut File, start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity(len as usize);
    file.take(len).read_to_end(&mut buf)?;
    Ok(buf)
}

/// 허용 여부·경로·Range 헤더(없으면 None)로 응답을 만든다. `full_limit` 은 Range 없는 전체 응답의 상한
/// (운영에서는 MAX_FULL_BYTES, 테스트에서는 작은 값을 주입한다).
pub fn build_response(
    allowed: bool,
    path: &str,
    range: Option<&str>,
    full_limit: u64,
) -> Response<Vec<u8>> {
    if !allowed {
        return error(StatusCode::FORBIDDEN);
    }
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return error(StatusCode::NOT_FOUND),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let len = match file.metadata() {
        Ok(m) if m.is_file() => m.len(),
        Ok(_) => return error(StatusCode::NOT_FOUND),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let mime = content_type(path);

    match range {
        Some(h) => match parse_range(h, len) {
            RangeResult::NotSatisfiable => base(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .header(header::CONTENT_TYPE, "text/plain")
                .body(b"Range Not Satisfiable".to_vec())
                .unwrap(),
            RangeResult::Partial { start, end } => {
                match read_span(&mut file, start, end - start + 1) {
                    Ok(body) => base(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, mime)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
                        .header(header::CONTENT_LENGTH, body.len())
                        .body(body)
                        .unwrap(),
                    Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR),
                }
            }
        },
        None if len > full_limit => error(StatusCode::PAYLOAD_TOO_LARGE),
        None => {
            // 전체를 Vec 에 올리기 전에 슬롯을 잡는다(응답을 만든 뒤 놓는다)
            let _slot = FULL_READS.acquire();
            match read_span(&mut file, 0, len) {
                Ok(body) => base(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::ACCEPT_RANGES, "bytes")
                    .header(header::CONTENT_LENGTH, body.len())
                    .body(body)
                    .unwrap(),
                Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tauri 결선
// ---------------------------------------------------------------------------

/// `Builder::register_asynchronous_uri_scheme_protocol("peek", peek::handle)` 로 등록한다.
/// `PreviewState` 를 `manage` 해 둬야 한다. 허용 검사는 요청 스레드에서 끝내고, 파일 읽기만 blocking 풀에서 한다.
pub fn handle<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    if request.method() == Method::OPTIONS {
        responder.respond(preflight());
        return;
    }
    let path = path_from_url(&request.uri().to_string());
    let allowed = match &path {
        Some(p) => ctx
            .app_handle()
            .state::<PreviewState>()
            .is_allowed(ctx.webview_label(), p),
        None => false,
    };
    let range = request.headers().get(header::RANGE).map(|v| {
        // 비 ASCII 헤더는 형식 오류로 취급해 416 이 되게 한다.
        v.to_str().unwrap_or("").to_owned()
    });
    let path = path.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        responder.respond(build_response(
            allowed,
            &path,
            range.as_deref(),
            MAX_FULL_BYTES,
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const MIB: u64 = 1024 * 1024;

    fn part(start: u64, end: u64) -> RangeResult {
        RangeResult::Partial { start, end }
    }

    // ---- Range ----

    #[test]
    fn range_세_형식() {
        assert_eq!(parse_range("bytes=0-3", 10), part(0, 3));
        assert_eq!(parse_range("bytes=4-", 10), part(4, 9));
        assert_eq!(parse_range("bytes=-3", 10), part(7, 9));
    }

    #[test]
    fn range_끝이_파일_밖이면_파일_끝으로_자른다() {
        assert_eq!(parse_range("bytes=5-999", 10), part(5, 9));
        assert_eq!(parse_range("bytes=-999", 10), part(0, 9));
        // u64 를 넘는 숫자도 포화돼 같은 결과
        assert_eq!(
            parse_range("bytes=5-99999999999999999999999", 10),
            part(5, 9)
        );
    }

    #[test]
    fn range_빈_파일은_항상_416() {
        for h in ["bytes=0-0", "bytes=0-", "bytes=-1"] {
            assert_eq!(parse_range(h, 0), RangeResult::NotSatisfiable, "{h}");
        }
    }

    #[test]
    fn range_마지막_바이트() {
        assert_eq!(parse_range("bytes=9-9", 10), part(9, 9));
        assert_eq!(parse_range("bytes=9-", 10), part(9, 9));
        assert_eq!(parse_range("bytes=-1", 10), part(9, 9));
    }

    #[test]
    fn range_범위_밖() {
        for h in ["bytes=10-", "bytes=10-12", "bytes=11-20", "bytes=-0"] {
            assert_eq!(parse_range(h, 10), RangeResult::NotSatisfiable, "{h}");
        }
    }

    #[test]
    fn range_역전은_416() {
        assert_eq!(parse_range("bytes=5-4", 10), RangeResult::NotSatisfiable);
    }

    #[test]
    fn range_다중_구간과_형식_오류는_416() {
        for h in [
            "bytes=0-1,3-4",
            "bytes=0-1, 3-",
            "bytes=",
            "bytes=-",
            "bytes=abc",
            "bytes=a-b",
            "bytes=1",
            "bytes=+1-3",
            "bytes=1--3",
            "bytes=0x1-3",
            "items=0-3",
            "0-3",
            "",
            "bytes",
            "=0-3",
        ] {
            assert_eq!(parse_range(h, 10), RangeResult::NotSatisfiable, "{h:?}");
        }
    }

    #[test]
    fn range_공백과_대소문자() {
        assert_eq!(parse_range("  bytes=0-3  ", 10), part(0, 3));
        assert_eq!(parse_range("BYTES=0-3", 10), part(0, 3));
        assert_eq!(parse_range("Bytes = 2 - 4", 10), part(2, 4));
        assert_eq!(parse_range("bytes=-3 ", 10), part(7, 9));
    }

    #[test]
    fn range_4mib_상한으로_자른다() {
        let len = 10 * MIB;
        let cap = 4 * MIB;
        assert_eq!(parse_range("bytes=0-", len), part(0, cap - 1));
        assert_eq!(parse_range("bytes=0-9999999", len), part(0, cap - 1));
        assert_eq!(parse_range("bytes=100-", len), part(100, 100 + cap - 1));
        // 정확히 상한인 구간은 그대로, 상한 + 1 은 잘린다
        assert_eq!(
            parse_range(&format!("bytes=0-{}", cap - 1), len),
            part(0, cap - 1)
        );
        assert_eq!(
            parse_range(&format!("bytes=0-{}", cap), len),
            part(0, cap - 1)
        );
        // 꼬리 구간이 상한보다 크면 시작점부터 상한만큼
        assert_eq!(
            parse_range(&format!("bytes=-{}", 8 * MIB), len),
            part(2 * MIB, 2 * MIB + cap - 1)
        );
        // 상한보다 작은 꼬리는 파일 끝까지
        assert_eq!(parse_range("bytes=-100", len), part(len - 100, len - 1));
        // 남은 길이가 상한보다 작으면 파일 끝까지
        assert_eq!(
            parse_range(&format!("bytes={}-", len - 10), len),
            part(len - 10, len - 1)
        );
    }

    // ---- URL ----

    /// JS encodeURIComponent: 영숫자와 -_.!~*'() 만 그대로 두고 나머지는 UTF-8 바이트 %XX
    fn encode_uri_component(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    fn long_path() -> String {
        format!("C:\\{}\\파일.txt", "가나다라마바사".repeat(40))
    }

    fn roundtrip_paths() -> Vec<String> {
        vec![
            r"C:\Users\user\한글 문서\보고서.docx".into(),
            r"C:\a\#hash.txt".into(),
            r"C:\a\100%.txt".into(),
            r"C:\a\1+1=2.txt".into(),
            r"C:\a\a b  c.txt".into(),
            r"C:\a\a&b=c.txt".into(),
            r"C:\a\what?.txt".into(),
            r"C:\a\%41%zz.txt".into(),
            r"C:\a\a?b#c%d+e f&g.txt".into(),
            r"\\server\share\a.txt".into(),
            r"\\?\C:\긴\경로\a.txt".into(),
            r"\\?\UNC\server\share\a.txt".into(),
            "C:\\a\\\u{00E9}.txt".into(),  // NFC
            "C:\\a\\e\u{0301}.txt".into(), // NFD
            "C:\\a\\😀.txt".into(),
            long_path(),
        ]
    }

    #[test]
    fn url_왕복_모든_접두사() {
        assert!(long_path().chars().count() > 260);
        for p in roundtrip_paths() {
            let enc = encode_uri_component(&p);
            for prefix in URL_PREFIXES {
                let url = format!("{prefix}{enc}");
                assert_eq!(path_from_url(&url).as_deref(), Some(p.as_str()), "{url}");
            }
        }
    }

    #[test]
    fn url_nfc_nfd_는_구별된다() {
        let nfc = "C:\\a\\\u{00E9}.txt";
        let nfd = "C:\\a\\e\u{0301}.txt";
        assert_ne!(nfc, nfd);
        let a = path_from_url(&format!(
            "http://peek.localhost/{}",
            encode_uri_component(nfc)
        ))
        .unwrap();
        let b = path_from_url(&format!(
            "http://peek.localhost/{}",
            encode_uri_component(nfd)
        ))
        .unwrap();
        assert_eq!(a, nfc);
        assert_eq!(b, nfd);
        assert_ne!(a, b);
    }

    #[test]
    fn url_더하기는_공백이_아니다() {
        assert_eq!(
            path_from_url("peek://localhost/a+b").as_deref(),
            Some("a+b")
        );
    }

    #[test]
    fn url_잘못된_퍼센트_인코딩은_none() {
        for u in [
            "peek://localhost/%",
            "peek://localhost/%4",
            "peek://localhost/%zz",
            "peek://localhost/a%G1",
            "peek://localhost/%FF",         // 단독 0xFF: UTF-8 아님
            "peek://localhost/%C3",         // 잘린 2바이트 시퀀스
            "peek://localhost/%C0%AF",      // 과길이 인코딩
            "peek://localhost/%ED%A0%80",   // 서로게이트 영역
            "http://peek.localhost/%E4%B8", // 잘린 3바이트 시퀀스
        ] {
            assert_eq!(path_from_url(u), None, "{u}");
        }
    }

    #[test]
    fn url_접두사가_다르거나_경로가_비면_none() {
        for u in [
            "",
            "peek://localhost",
            "peek://localhost/",
            "peek://file/a",
            "http://peek.localhost",
            "http://peek.localhost/",
            "http://peek.localhost/?x=1",
            "http://other.localhost/a",
            "ftp://peek.localhost/a",
            "http://peek.localhost.evil.com/a",
            "http://peek.localhostx/a",
            "ünicode",
        ] {
            assert_eq!(path_from_url(u), None, "{u:?}");
        }
    }

    #[test]
    fn url_날것의_쿼리는_거부하고_프래그먼트는_디코딩_전에_잘린다() {
        // `?` 는 encodeURIComponent 출력에 날것으로 나오지 않는다
        for u in [
            "peek://localhost/a?x=1",
            "peek://localhost/a%3Fb?x=1",
            "peek://localhost/a?x#y",
            "peek://localhost/a?%zz",
            "peek://localhost/a?",
        ] {
            assert_eq!(path_from_url(u), None, "{u}");
        }
        // 인코딩된 `?`·`#` 는 경로의 일부
        assert_eq!(
            path_from_url("peek://localhost/a%3Fb").as_deref(),
            Some("a?b")
        );
        assert_eq!(
            path_from_url("peek://localhost/a%23b#frag").as_deref(),
            Some("a#b")
        );
        assert_eq!(path_from_url("peek://localhost/a#y").as_deref(), Some("a"));
        // 프래그먼트 뒤는 보지 않는다
        assert_eq!(
            path_from_url("peek://localhost/a#%zz?x").as_deref(),
            Some("a")
        );
    }

    #[test]
    fn url_스킴과_호스트는_대소문자_무시() {
        assert_eq!(
            path_from_url("HTTP://Peek.Localhost/a").as_deref(),
            Some("a")
        );
        // 경로 부분은 그대로 보존
        assert_eq!(
            path_from_url("peek://localhost/%41b").as_deref(),
            Some("Ab")
        );
    }

    // ---- 전체 읽기 동시 상한 ----

    #[test]
    fn 전체_읽기는_동시에_상한까지만() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        let limiter = Arc::new(FullReadLimiter::new(2));
        let running = Arc::new(AtomicU32::new(0));
        let peak = Arc::new(AtomicU32::new(0));
        let threads: Vec<_> = (0..3)
            .map(|_| {
                let (limiter, running, peak) = (limiter.clone(), running.clone(), peak.clone());
                std::thread::spawn(move || {
                    let _slot = limiter.acquire();
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(100));
                    running.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        // 세 요청이 모두 끝났고(기다리던 것도 진행) 동시 실행은 2 를 넘지 않았다
        assert!(peak.load(Ordering::SeqCst) <= 2);
        // 슬롯이 모두 돌아왔다
        let a = limiter.acquire();
        let b = limiter.acquire();
        drop((a, b));
    }

    // ---- 응답 ----

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str, data: &[u8]) -> Self {
            let dir =
                std::env::temp_dir().join(format!("peekaboo-peek-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let p = dir.join(name);
            std::fs::write(&p, data).unwrap();
            Temp(p)
        }

        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn h<'a>(r: &'a Response<Vec<u8>>, name: &str) -> Option<&'a str> {
        r.headers().get(name).map(|v| v.to_str().unwrap())
    }

    fn assert_common(r: &Response<Vec<u8>>) {
        assert_eq!(h(r, "access-control-allow-origin"), Some("*"));
        assert_eq!(
            h(r, "access-control-expose-headers"),
            Some("Content-Range, Content-Length, Accept-Ranges")
        );
        assert_eq!(h(r, "cache-control"), Some("no-store"));
        assert_eq!(
            h(r, "content-security-policy"),
            Some("sandbox; default-src 'none'")
        );
    }

    #[test]
    fn 응답_200_전체() {
        let f = Temp::new("full.txt", b"0123456789");
        let r = build_response(true, f.path(), None, MAX_FULL_BYTES);
        assert_eq!(r.status(), 200);
        assert_eq!(r.body(), b"0123456789");
        assert_eq!(h(&r, "content-length"), Some("10"));
        assert_eq!(h(&r, "accept-ranges"), Some("bytes"));
        assert_eq!(h(&r, "content-type"), Some("text/plain"));
        assert_common(&r);
    }

    #[test]
    fn 응답_200_빈_파일() {
        let f = Temp::new("empty.bin", b"");
        let r = build_response(true, f.path(), None, MAX_FULL_BYTES);
        assert_eq!(r.status(), 200);
        assert!(r.body().is_empty());
        assert_eq!(h(&r, "content-type"), Some("application/octet-stream"));
    }

    #[test]
    fn 응답_206_구간() {
        let f = Temp::new("part.mp4", b"0123456789");
        for (range, body, cr) in [
            ("bytes=2-5", &b"2345"[..], "bytes 2-5/10"),
            ("bytes=7-", b"789", "bytes 7-9/10"),
            ("bytes=-2", b"89", "bytes 8-9/10"),
            ("bytes=8-100", b"89", "bytes 8-9/10"),
        ] {
            let r = build_response(true, f.path(), Some(range), MAX_FULL_BYTES);
            assert_eq!(r.status(), 206, "{range}");
            assert_eq!(r.body(), body, "{range}");
            assert_eq!(h(&r, "content-range"), Some(cr), "{range}");
            assert_eq!(
                h(&r, "content-length"),
                Some(body.len().to_string().as_str())
            );
            assert_eq!(h(&r, "accept-ranges"), Some("bytes"));
            assert_eq!(h(&r, "content-type"), Some("video/mp4"));
            assert_common(&r);
        }
    }

    #[test]
    fn 응답_206_은_4mib_로_잘린다() {
        let data = vec![7u8; (5 * MIB) as usize];
        let f = Temp::new("big.bin", &data);
        let r = build_response(true, f.path(), Some("bytes=0-"), MAX_FULL_BYTES);
        assert_eq!(r.status(), 206);
        assert_eq!(r.body().len() as u64, 4 * MIB);
        assert_eq!(
            h(&r, "content-range"),
            Some(format!("bytes 0-{}/{}", 4 * MIB - 1, 5 * MIB).as_str())
        );
    }

    #[test]
    fn 응답_416() {
        let f = Temp::new("r416.txt", b"0123456789");
        for range in ["bytes=10-", "bytes=5-4", "bytes=0-1,3-4", "garbage", ""] {
            let r = build_response(true, f.path(), Some(range), MAX_FULL_BYTES);
            assert_eq!(r.status(), 416, "{range:?}");
            assert_eq!(h(&r, "content-range"), Some("bytes */10"));
            assert_common(&r);
        }
        let e = Temp::new("r416-empty.txt", b"");
        assert_eq!(
            build_response(true, e.path(), Some("bytes=0-"), MAX_FULL_BYTES).status(),
            416
        );
    }

    #[test]
    fn 응답_403_은_파일_존재_여부보다_먼저() {
        let f = Temp::new("deny.txt", b"x");
        let r = build_response(false, f.path(), None, MAX_FULL_BYTES);
        assert_eq!(r.status(), 403);
        assert_common(&r);
        // 없는 파일이어도 403(존재 여부를 흘리지 않는다)
        assert_eq!(
            build_response(false, r"C:\no\such\file", None, MAX_FULL_BYTES).status(),
            403
        );
    }

    #[test]
    fn 응답_404() {
        let r = build_response(true, r"C:\no\such\peekaboo-file.txt", None, MAX_FULL_BYTES);
        assert_eq!(r.status(), 404);
        assert_common(&r);
        // 폴더도 파일이 아니므로 404
        let dir = std::env::temp_dir();
        assert_eq!(
            build_response(true, dir.to_str().unwrap(), None, MAX_FULL_BYTES).status(),
            404
        );
    }

    #[test]
    fn 응답_413_은_range_없는_전체_요청만() {
        let f = Temp::new("limit.bin", b"0123456789");
        let r = build_response(true, f.path(), None, 9);
        assert_eq!(r.status(), 413);
        assert_common(&r);
        // 상한과 같으면 통과
        assert_eq!(build_response(true, f.path(), None, 10).status(), 200);
        // Range 가 있으면 상한과 무관하게 206
        assert_eq!(
            build_response(true, f.path(), Some("bytes=0-3"), 9).status(),
            206
        );
    }

    #[test]
    fn 응답_한글_경로_파일() {
        let f = Temp::new("보고서 #1 100%.txt", "안녕".as_bytes());
        let p = path_from_url(&format!(
            "http://peek.localhost/{}",
            encode_uri_component(f.path())
        ))
        .unwrap();
        assert_eq!(p, f.path());
        let r = build_response(true, &p, None, MAX_FULL_BYTES);
        assert_eq!(r.status(), 200);
        assert_eq!(r.body(), "안녕".as_bytes());
    }

    #[test]
    fn preflight_응답() {
        let r = preflight();
        assert_eq!(r.status(), 204);
        assert_eq!(h(&r, "access-control-allow-headers"), Some("Range"));
        assert_common(&r);
    }

    #[test]
    fn content_type_표() {
        assert_eq!(content_type(r"C:\a\X.PDF"), "application/pdf");
        assert_eq!(content_type(r"C:\a.b\noext"), "application/octet-stream");
        assert_eq!(content_type(r"C:\a\x.JPG"), "image/jpeg");
        assert_eq!(content_type(r"C:\a\x.unknown"), "application/octet-stream");
    }
}
