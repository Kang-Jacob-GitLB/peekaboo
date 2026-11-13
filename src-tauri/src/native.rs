// M0 판정 4번: 헬퍼(C#)의 Windows 미리보기 핸들러 컨테이너가 WebView2 위에 깜박임 없이 겹치고
// 리사이즈를 따라가는지 본다. 입력은 보내지 않고, 우리 창만 캡처한다.
//
// 환경 변수: PEEKABOO_SPIKE_FILE(미리볼 파일), PEEKABOO_SPIKE_HELPER(peekaboo-helper.exe),
//            PEEKABOO_SPIKE_OUT(캡처·헬퍼 stderr 를 둘 폴더, 기본은 임시 폴더)
use std::ffi::c_void;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::sleep;
use std::time::{Duration, Instant};

use tauri::{AppHandle, PhysicalSize, WebviewWindow};
use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, GetAncestor, GetClassNameW, GetClientRect, GetDesktopWindow, GetWindow,
    GetWindowRect, GetWindowThreadProcessId, SetWindowPos, WindowFromPoint, GA_PARENT, GW_CHILD, GW_HWNDNEXT,
    HWND_TOP, PW_RENDERFULLCONTENT, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

use crate::{foreground, Log};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 헬퍼 컨테이너 창 클래스(helper/Shell/NativePreviewHost.cs 의 ContainerClass)
const CONTAINER_CLASS: &str = "PeekabooPreviewContainer";
/// 렌더러 헤더 높이(물리 픽셀). 이 아래가 네이티브 영역
const HEADER: i32 = 48;
const PREVIEW_ID: i32 = 1;

#[derive(Default, Clone, Copy)]
struct Tally {
    native: u32,
    webview: u32,
    total: u32,
}

struct WinInfo {
    hwnd: isize,
    class: String,
    pid: u32,
}

fn hw(h: isize) -> HWND {
    HWND(h as *mut c_void)
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn info(h: HWND) -> WinInfo {
    WinInfo { hwnd: h.0 as isize, class: class_of(h), pid: pid_of(h) }
}

fn proc_name(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return "?".into();
        };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return "?".into();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit('\\').next().unwrap_or("?").to_string()
    }
}

/// 맞은 창에서 루트까지 부모 체인(자식 → 부모 순). 최상위 창은 소유자가 아니라 데스크톱에서 멈춘다.
fn chain_of(hit: HWND) -> Vec<WinInfo> {
    let desktop = unsafe { GetDesktopWindow() };
    let mut chain = vec![info(hit)];
    let mut cur = hit;
    for _ in 0..32 {
        let parent = unsafe { GetAncestor(cur, GA_PARENT) };
        if parent.0.is_null() || parent == desktop {
            break;
        }
        chain.push(info(parent));
        cur = parent;
    }
    chain
}

fn classify(chain: &[WinInfo]) -> &'static str {
    if chain.iter().any(|w| w.class == CONTAINER_CLASS) {
        "NATIVE_ON_TOP"
    } else if chain
        .iter()
        .any(|w| w.class.starts_with("Chrome_") || w.class == "WRY_WEBVIEW" || proc_name(w.pid).contains("msedgewebview2"))
    {
        "WEBVIEW_ON_TOP"
    } else {
        "OTHER"
    }
}

fn client_rect(top: HWND) -> RECT {
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(top, &mut rc);
    }
    rc
}

/// 네이티브 영역(클라이언트 좌표): 헤더 아래 나머지 전체
fn native_rect(top: HWND) -> RECT {
    let rc = client_rect(top);
    RECT { left: 0, top: HEADER, right: rc.right, bottom: rc.bottom }
}

fn rect_json(r: &RECT) -> String {
    format!("{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}", r.left, r.top, r.right - r.left, r.bottom - r.top)
}

/// 클라이언트 좌표 → 화면 좌표
fn to_screen(top: HWND, x: i32, y: i32) -> POINT {
    let mut p = POINT { x, y };
    unsafe {
        let _ = ClientToScreen(top, &mut p);
    }
    p
}

/// 네이티브 영역 중앙과 네 모서리 안쪽 20px
fn sample_points(top: HWND) -> [(&'static str, POINT); 5] {
    let r = native_rect(top);
    let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    [
        ("center", to_screen(top, cx, cy)),
        ("top-left", to_screen(top, r.left + 20, r.top + 20)),
        ("top-right", to_screen(top, r.right - 20, r.top + 20)),
        ("bottom-left", to_screen(top, r.left + 20, r.bottom - 20)),
        ("bottom-right", to_screen(top, r.right - 20, r.bottom - 20)),
    ]
}

/// 우리 창의 직접 자식을 Z 순서(위 → 아래)로 나열
fn children_z(top: HWND) -> String {
    let mut out = Vec::new();
    let mut cur = unsafe { GetWindow(top, GW_CHILD) }.ok();
    while let Some(h) = cur {
        if h.0.is_null() || out.len() > 32 {
            break;
        }
        let i = info(h);
        out.push(format!("0x{:X}:{}:pid{}", i.hwnd, i.class, i.pid));
        cur = unsafe { GetWindow(h, GW_HWNDNEXT) }.ok();
    }
    out.join(" > ")
}

fn sample(log: &Log, tag: &str, top: HWND) -> Tally {
    let mut t = Tally::default();
    log.line(&format!("sample[{tag}] children(top->bottom)={}", children_z(top)));
    for (name, p) in sample_points(top) {
        let hit = unsafe { WindowFromPoint(p) };
        let chain = chain_of(hit);
        let verdict = classify(&chain);
        let hit_info = &chain[0];
        let chain_s: Vec<String> =
            chain.iter().map(|w| format!("0x{:X}:{}:pid{}", w.hwnd, w.class, w.pid)).collect();
        log.line(&format!(
            "sample[{tag}] {name} ({},{}) hit=0x{:X} class={} pid={} proc={} chain={} => {verdict}",
            p.x,
            p.y,
            hit_info.hwnd,
            hit_info.class,
            hit_info.pid,
            proc_name(hit_info.pid),
            chain_s.join(" <- ")
        ));
        t.total += 1;
        match verdict {
            "NATIVE_ON_TOP" => t.native += 1,
            "WEBVIEW_ON_TOP" => t.webview += 1,
            _ => {}
        }
    }
    t
}

/// 우리 최상위 창을 PrintWindow(PW_RENDERFULLCONTENT)로 PNG 저장한다. 화면 전체 캡처가 아니다.
fn capture(log: &Log, top: HWND, path: &Path) {
    let mut rc = RECT::default();
    if unsafe { GetWindowRect(top, &mut rc) }.is_err() {
        log.line("capture: GetWindowRect 실패");
        return;
    }
    let (w, h) = ((rc.right - rc.left).max(1), (rc.bottom - rc.top).max(1));
    unsafe {
        let hdc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 위에서 아래
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            log.line("capture: CreateDIBSection 실패");
            let _ = DeleteDC(hdc);
            return;
        };
        let old = SelectObject(hdc, HGDIOBJ(bmp.0));
        let ok = PrintWindow(top, hdc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();
        let raw = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
        let mut rgba = Vec::with_capacity(raw.len());
        for px in raw.as_chunks::<4>().0 {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        SelectObject(hdc, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(hdc);

        let saved = std::fs::File::create(path).ok().and_then(|f| {
            let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().ok()?.write_image_data(&rgba).ok()
        });
        log.line(&format!(
            "capture {} printwindow_ok={ok} size={w}x{h} saved={}",
            path.display(),
            saved.is_some()
        ));
    }
}

/// 우리 창 자손 중 헬퍼 컨테이너(헬퍼 PID 소유)
fn find_container(top: HWND, helper_pid: u32) -> Option<HWND> {
    struct Ctx {
        pid: u32,
        found: Option<isize>,
    }
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let ctx = unsafe { &mut *(lp.0 as *mut Ctx) };
        if pid_of(h) == ctx.pid && class_of(h) == CONTAINER_CLASS {
            ctx.found = Some(h.0 as isize);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut ctx = Ctx { pid: helper_pid, found: None };
    unsafe {
        let _ = EnumChildWindows(Some(top), Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.found.map(hw)
}

/// 헬퍼 stdout 한 줄에서 `"key":"value"` 의 문자열 값(이스케이프 없는 단순 값만)
fn json_str(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let end = line[start..].find('"')?;
    Some(line[start..start + end].to_string())
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn send(log: &Log, stdin: &mut ChildStdin, line: &str) {
    log.line(&format!("-> helper {line}"));
    if writeln!(stdin, "{line}").and_then(|_| stdin.flush()).is_err() {
        log.line("helper stdin 쓰기 실패");
    }
}

/// 마감 시각까지 헬퍼 메시지를 받아 로그하고, nativePreview 응답이 오면 status 를 채운다.
fn pump(log: &Log, rx: &Receiver<String>, deadline: Instant, status: &mut Option<String>) {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) => {
                log.line(&format!("<- helper {line}"));
                if json_str(&line, "type").as_deref() == Some("nativePreview") {
                    *status = json_str(&line, "status");
                }
            }
            Err(_) => return,
        }
    }
}

fn spawn_helper(exe: &str, stderr_path: &Path) -> std::io::Result<(Child, Receiver<String>)> {
    let stderr = std::fs::File::create(stderr_path)?;
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Ok((child, rx))
}

fn sizes_match(top: HWND, container: Option<HWND>) -> bool {
    let Some(c) = container else { return false };
    let want = native_rect(top);
    let mut rc = RECT::default();
    if unsafe { GetWindowRect(c, &mut rc) }.is_err() {
        return false;
    }
    (rc.right - rc.left, rc.bottom - rc.top) == (want.right - want.left, want.bottom - want.top)
}

pub fn run(app: &AppHandle, window: &WebviewWindow, log: &Log, our_hwnd: isize) {
    let result = |status: &str, t: Tally, webview: u32, resized_ok: bool| {
        log.line(&format!(
            "RESULT mode=native status={status} on_top={}/{} webview_on_top={webview} resized_ok={resized_ok}",
            t.native, t.total
        ));
    };
    let top = hw(our_hwnd);
    let out_dir = std::env::var_os("PEEKABOO_SPIKE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&out_dir);
    let (Some(file), Some(helper)) =
        (std::env::var("PEEKABOO_SPIKE_FILE").ok(), std::env::var("PEEKABOO_SPIKE_HELPER").ok())
    else {
        log.line("PEEKABOO_SPIKE_FILE / PEEKABOO_SPIKE_HELPER 가 필요하다");
        result("config_missing", Tally::default(), 0, false);
        app.exit(1);
        return;
    };

    // 잠금 화면이면 아무것도 띄우지 않는다
    let fg = foreground();
    let fg_class = class_of(hw(fg.hwnd));
    log.line(&format!("foreground hwnd=0x{:X} pid={} class={fg_class} title={:?}", fg.hwnd, fg.pid, fg.title));
    if fg_class == "Windows.UI.Core.CoreWindow" || fg.title.contains("잠금") {
        log.line("전경 창이 잠금 화면이라 실행하지 않는다");
        result("locked_screen", Tally::default(), 0, false);
        app.exit(2);
        return;
    }

    // 웹뷰 로드 대기 후 표시. 이후 시각은 show 기준
    sleep(Duration::from_secs(2));
    let _ = window.show();
    let t0 = Instant::now();
    let rc = client_rect(top);
    log.line(&format!("shown client={}x{} children={}", rc.right, rc.bottom, children_z(top)));

    let (mut child, rx) = match spawn_helper(&helper, &out_dir.join("helper-stderr.log")) {
        Ok(v) => v,
        Err(e) => {
            log.line(&format!("헬퍼 실행 실패: {e}"));
            result("spawn_failed", Tally::default(), 0, false);
            app.exit(1);
            return;
        }
    };
    let helper_pid = child.id();
    let mut stdin = child.stdin.take().expect("piped stdin");
    log.line(&format!("helper pid={helper_pid}"));

    // ready 대기(최대 10초). 이 동안에도 0.5s 샘플 시각을 넘기지 않도록 짧게 끊어 로그만 받는다
    let mut status: Option<String> = None;
    let mut ready = false;
    let ready_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < ready_deadline && !ready {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(100)) {
            log.line(&format!("<- helper {line}"));
            ready = json_str(&line, "type").as_deref() == Some("ready");
        }
    }
    if !ready {
        log.line("ready 를 받지 못함");
        let _ = child.kill();
        result("no_ready", Tally::default(), 0, false);
        app.exit(1);
        return;
    }

    // 설치된 Electron Peekaboo 와 훅이 겹치지 않게 키보드 가로채기를 끈다
    send(log, &mut stdin, &format!("{{\"v\":1,\"type\":\"config\",\"enabled\":false,\"hostPid\":{}}}", std::process::id()));
    let rect = native_rect(top);
    send(
        log,
        &mut stdin,
        &format!(
            "{{\"v\":1,\"type\":\"nativePreview.open\",\"id\":{PREVIEW_ID},\"path\":\"{}\",\"hwnd\":{our_hwnd},\"rect\":{},\"dark\":false}}",
            json_escape(&file),
            rect_json(&rect)
        ),
    );
    let t_open = Instant::now();

    let mut total = Tally::default();
    let mut webview_total = 0u32;
    let add = |t: Tally, total: &mut Tally, webview_total: &mut u32| {
        total.native += t.native;
        total.webview += t.webview;
        total.total += t.total;
        *webview_total += t.webview;
    };

    for (ms, tag) in [(500u64, "0.5s"), (1500, "1.5s"), (3000, "3s"), (5000, "5s")] {
        pump(log, &rx, t0 + Duration::from_millis(ms), &mut status);
        log.line(&format!("-- {tag} (open 응답 status={:?}, open 후 {}ms)", status, t_open.elapsed().as_millis()));
        let t = sample(log, tag, top);
        add(t, &mut total, &mut webview_total);
        capture(log, top, &out_dir.join(format!("cap-{tag}.png")));
    }

    // open 응답이 아직이면 더 기다린다(Office 서버 프로세스 기동이 길 수 있음)
    let status_deadline = Instant::now() + Duration::from_secs(8);
    while status.is_none() && Instant::now() < status_deadline {
        pump(log, &rx, Instant::now() + Duration::from_millis(200), &mut status);
    }
    let status_s = status.clone().unwrap_or_else(|| "no_response".into());

    // 리사이즈 시험
    log.line("-- resize 1100x760");
    window.set_size(PhysicalSize::new(1100u32, 760u32)).ok();
    pump(log, &rx, Instant::now() + Duration::from_millis(300), &mut status);
    let new_rect = native_rect(top);
    log.line(&format!("client after resize={}x{}", client_rect(top).right, client_rect(top).bottom));
    send(
        log,
        &mut stdin,
        &format!("{{\"v\":1,\"type\":\"nativePreview.resize\",\"id\":{PREVIEW_ID},\"rect\":{}}}", rect_json(&new_rect)),
    );
    pump(log, &rx, Instant::now() + Duration::from_secs(1), &mut status);
    let t = sample(log, "resized", top);
    let container = find_container(top, helper_pid);
    let size_ok = sizes_match(top, container);
    log.line(&format!("container={:?} size_matches_native_rect={size_ok}", container.map(|c| c.0 as isize)));
    capture(log, top, &out_dir.join("cap-resized.png"));
    let resized_ok = t.native == t.total && size_ok;
    add(t, &mut total, &mut webview_total);

    // A안: WEBVIEW_ON_TOP 이 한 번이라도 나왔으면 컨테이너를 HWND_TOP 으로 올려 본다
    let a_needed = webview_total > 0;
    if a_needed {
        match find_container(top, helper_pid) {
            Some(c) => {
                log.line(&format!("A안 시험: SetWindowPos(0x{:X}, HWND_TOP)", c.0 as isize));
                unsafe {
                    let _ = SetWindowPos(c, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
                sleep(Duration::from_millis(500));
                let t = sample(log, "A_applied", top);
                capture(log, top, &out_dir.join("cap-A.png"));
                log.line(&format!("A안 결과 native={}/{} webview={}", t.native, t.total, t.webview));
            }
            None => log.line("A안 시험 불가: 컨테이너를 찾지 못함"),
        }
    } else {
        log.line("A안 불필요: WEBVIEW_ON_TOP 없음");
    }

    // 종료 절차
    send(log, &mut stdin, &format!("{{\"v\":1,\"type\":\"nativePreview.close\",\"id\":{PREVIEW_ID}}}"));
    pump(log, &rx, Instant::now() + Duration::from_millis(500), &mut status);
    send(log, &mut stdin, "{\"v\":1,\"type\":\"shutdown\"}");
    let kill_at = Instant::now() + Duration::from_secs(2);
    let exited = loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break true;
        }
        if Instant::now() >= kill_at {
            break false;
        }
        sleep(Duration::from_millis(50));
    };
    if !exited {
        log.line("헬퍼가 2초 안에 끝나지 않아 kill");
        let _ = child.kill();
        let _ = child.wait();
    }
    log.line(&format!("helper exited_cleanly={exited}"));
    result(&status_s, total, webview_total, resized_ok);
    app.exit(0);
}
