// M0 스파이크 판정 1번: 프레임 없는 항상-위 창을 show → hide → show 반복해도
// 포그라운드 창(포커스)을 뺏지 않는지 확인한다. 렌더러·peek 프로토콜·헬퍼는 붙이지 않는다.
// 판정 4번(PEEKABOO_SPIKE_MODE=native): 헬퍼의 미리보기 핸들러 창이 WebView2 위에 겹치는지는 native.rs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("이 스파이크는 Windows 전용이다");

mod native;
mod peek;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::Duration;

use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, SetWindowPos, ShowWindow, WindowFromPoint,
    GA_ROOT, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SW_HIDE, WS_EX_NOACTIVATE,
};

const LOOPS: u32 = 5;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Tao,
    Hwnd,
    Native,
}

impl Mode {
    fn from_env() -> Self {
        match std::env::var("PEEKABOO_SPIKE_MODE").as_deref() {
            Ok("hwnd") => Mode::Hwnd,
            Ok("native") => Mode::Native,
            _ => Mode::Tao,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Tao => "tao",
            Mode::Hwnd => "hwnd",
            Mode::Native => "native",
        }
    }
}

/// stdout 과 파일에 같은 줄을 남긴다.
pub(crate) struct Log(Mutex<Option<File>>);

impl Log {
    fn open() -> Self {
        let path = std::env::var_os("PEEKABOO_SPIKE_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("peekaboo-spike.log"));
        Log(Mutex::new(File::create(path).ok()))
    }

    pub(crate) fn line(&self, msg: &str) {
        println!("{msg}");
        if let Some(f) = self.0.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{msg}");
        }
    }
}

pub(crate) struct Foreground {
    pub(crate) hwnd: isize,
    pub(crate) pid: u32,
    pub(crate) title: String,
}

pub(crate) fn foreground() -> Foreground {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        Foreground {
            hwnd: hwnd.0 as isize,
            pid,
            title: String::from_utf16_lossy(&buf[..len]),
        }
    }
}

fn sample(log: &Log, label: &str, iter: u32, our_hwnd: isize) -> Foreground {
    let fg = foreground();
    let (visible, noactivate, on_top) = our_state(our_hwnd);
    log.line(&format!(
        "iter={iter} {label} fg_hwnd=0x{:X} fg_pid={} fg_title={:?} our_hwnd=0x{:X} our_pid={} visible={visible} noactivate={noactivate} center_hit_ours={on_top}",
        fg.hwnd,
        fg.pid,
        fg.title,
        our_hwnd,
        std::process::id()
    ));
    fg
}

/// 우리 창의 실제 상태: 보이는지, WS_EX_NOACTIVATE 인지, 창 중앙 점을 맞히는 최상위 창이 우리 창인지.
/// 창이 아예 안 떴는데 포커스 유지로 오판하는 것을 막는다.
fn our_state(our_hwnd: isize) -> (bool, bool, bool) {
    let hwnd = HWND(our_hwnd as *mut c_void);
    unsafe {
        let visible = IsWindowVisible(hwnd).as_bool();
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let noactivate = ex & WS_EX_NOACTIVATE.0 != 0;
        let mut rect = RECT::default();
        let on_top = if GetWindowRect(hwnd, &mut rect).is_ok() {
            let center = POINT {
                x: (rect.left + rect.right) / 2,
                y: (rect.top + rect.bottom) / 2,
            };
            let hit = WindowFromPoint(center);
            let root = GetAncestor(hit, GA_ROOT);
            let mut pid = 0u32;
            GetWindowThreadProcessId(root, Some(&mut pid));
            let mut cls = [0u16; 128];
            let n = GetClassNameW(root, &mut cls) as usize;
            eprintln!(
                "  hit=0x{:X} root=0x{:X} root_pid={pid} root_class={} rect=({},{})-({},{})",
                hit.0 as isize,
                root.0 as isize,
                String::from_utf16_lossy(&cls[..n]),
                rect.left,
                rect.top,
                rect.right,
                rect.bottom
            );
            root == hwnd
        } else {
            false
        };
        (visible, noactivate, on_top)
    }
}

/// UI 스레드에서 클로저를 실행하고 끝날 때까지 기다린다.
fn on_main(app: &AppHandle, f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = mpsc::channel();
    let _ = app.run_on_main_thread(move || {
        f();
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(5));
}

fn show(app: &AppHandle, window: &WebviewWindow, mode: Mode, hwnd: isize) {
    match mode {
        Mode::Tao => {
            let _ = window.show();
        }
        Mode::Native => {}
        Mode::Hwnd => on_main(app, move || unsafe {
            let _ = SetWindowPos(
                HWND(hwnd as _),
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }),
    }
}

fn hide(app: &AppHandle, window: &WebviewWindow, mode: Mode, hwnd: isize) {
    match mode {
        Mode::Tao => {
            let _ = window.hide();
        }
        Mode::Native => {}
        Mode::Hwnd => on_main(app, move || unsafe {
            let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
        }),
    }
}

fn run_test(app: AppHandle, window: WebviewWindow) {
    let log = Log::open();
    let mode = Mode::from_env();
    let our_pid = std::process::id();
    let our_hwnd = window.hwnd().map(|h| h.0 as isize).unwrap_or(0);
    log.line(&format!("START mode={} loops={LOOPS} our_pid={our_pid}", mode.name()));
    if mode == Mode::Native {
        native::run(&app, &window, &log, our_hwnd);
        return;
    }

    // 웹뷰 로드 대기
    sleep(Duration::from_secs(2));

    let (mut ok, mut stolen) = (0u32, 0u32);
    for i in 1..=LOOPS {
        let before = sample(&log, "before_show", i, our_hwnd);
        show(&app, &window, mode, our_hwnd);

        sleep(Duration::from_millis(300));
        let a = sample(&log, "show+300ms", i, our_hwnd);
        sleep(Duration::from_millis(1100));
        let b = sample(&log, "show+1400ms", i, our_hwnd);
        sleep(Duration::from_millis(100));

        let is_stolen = [&a, &b]
            .iter()
            .any(|fg| fg.pid == our_pid || fg.pid != before.pid);
        if is_stolen {
            stolen += 1;
        } else {
            ok += 1;
        }
        log.line(&format!(
            "iter={i} verdict={}",
            if is_stolen { "FOCUS_STOLEN" } else { "OK" }
        ));

        hide(&app, &window, mode, our_hwnd);
        sleep(Duration::from_secs(1));
    }

    log.line(&format!("RESULT mode={} ok={ok} stolen={stolen}", mode.name()));
    app.exit(0);
}

fn main() {
    tauri::Builder::default()
        // peek 프로토콜: 허용 상태가 비어 있어 스파이크에서는 모든 요청이 403 이다.
        .manage(peek::AllowState::default())
        .register_asynchronous_uri_scheme_protocol("peek", peek::handle)
        .setup(|app| {
            // setup 은 동기 커맨드·이벤트 핸들러가 아니라 앱 초기화 단계라 창 생성이 안전하다.
            // (교착 경고는 동기 커맨드/이벤트 핸들러 한정 — WebviewWindowBuilder::new 문서)
            let native = Mode::from_env() == Mode::Native;
            let url = if native { "index.html?native" } else { "index.html" };
            let (width, height) = if native { (900.0, 640.0) } else { (640.0, 420.0) };
            let window = WebviewWindowBuilder::new(app, "preview", WebviewUrl::App(url.into()))
            .title("PeekabooSpike")
            .decorations(false)
            .always_on_top(true)
            .focusable(false)
            .skip_taskbar(true)
            .visible(false)
            .shadow(false)
            .resizable(false)
            .inner_size(width, height)
            .position(200.0, 200.0)
            .data_directory(std::env::temp_dir().join("peekaboo-spike-webview"))
            .build()?;

            let handle = app.handle().clone();
            std::thread::spawn(move || run_test(handle, window));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 앱 실행 실패");
}
