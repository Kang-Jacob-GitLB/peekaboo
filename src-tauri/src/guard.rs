// WebView2 COM 로 구현하는 보호·편의 기능. Electron 이 그냥 주던 것을 webview2-com 으로 직접 붙인다(docs/tauri-migration.md 2절).
//
// - iframe 이동 차단: `add_FrameNavigationStarting` 으로 `about:` 이외의 서브프레임 이동을 취소한다(Electron `will-frame-navigate`).
//   `ICoreWebView2::add_FrameNavigationStarting` 은 `ICoreWebView2NavigationStartingEventHandler` 를 받는다
//   (webview2-com 의 `FrameNavigationStartingEventHandler` 는 `ICoreWebView2Frame` 용이라 쓰지 않는다).
// - 브라우저 가속키 끄기: `ICoreWebView2Settings3::SetAreBrowserAcceleratorKeysEnabled(false)`
//   (Electron 의 `Menu.setApplicationMenu(null)`. Ctrl+R 새로 고침·Ctrl+P 인쇄·F5·확대/축소 등이 포커스를 받은 창에서 먹지 않게).
// - 우클릭 복사: `ICoreWebView2_11::add_ContextMenuRequested` 로 메뉴를 항상 숨기고, 선택한 텍스트가 있으면 호스트가 클립보드에 쓴다.
//   창이 포커스를 갖지 않아 팝업 메뉴가 바깥 클릭으로 닫히지 않을 수 있고, 렌더러의 navigator.clipboard 는 포커스가 없으면 거부된다.
// - 프로세스 실패(`add_ProcessFailed`): 렌더러가 죽으면 로그를 남기고 ready 를 내리고 열려 있던 미리보기를 닫고 페이지를 다시 불러온다.
//
// 모든 콜백은 WebView2 를 만든 UI 스레드에서 불린다.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Controller, ICoreWebView2ProcessFailedEventArgs2,
    ICoreWebView2Settings3, ICoreWebView2_11, COREWEBVIEW2_PROCESS_FAILED_KIND,
    COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    COREWEBVIEW2_PROCESS_FAILED_REASON,
};
use webview2_com::{
    take_pwstr, ContextMenuRequestedEventHandler, NavigationStartingEventHandler,
    ProcessFailedEventHandler,
};
use windows::core::{Interface, PWSTR};

use crate::controller::Controller;
use crate::preview::{self, PreviewState};

// ---------------------------------------------------------------------------
// 순수 판정(cargo test 로 검증)
// ---------------------------------------------------------------------------

/// 서브프레임이 이동해도 되는 주소. HTML 미리보기 iframe(srcdoc)은 `about:srcdoc` 이다.
/// 링크·`<area>`·`<base>` 등으로 다른 문서로 옮겨 가는 것은 막는다(Electron 의 `!url.startsWith('about:')` 와 같다).
fn allow_frame_navigation(uri: &str) -> bool {
    uri.starts_with("about:")
}

/// 프로세스 실패 종류별 대응
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureAction {
    /// 렌더러가 죽었다: ready 를 내리고, 열려 있던 미리보기를 닫고, 페이지를 다시 불러온다
    RecoverRenderer,
    /// 로그만. 무응답(Electron 도 `unresponsive` 는 로그만 남긴다: 기다리면 돌아올 수 있다), 하위 프레임·GPU·유틸리티 프로세스 종료
    /// (메인 페이지는 살아 있다). 브라우저 프로세스 종료는 WebView2 전체가 죽은 것이라 페이지를 다시 불러서는 살릴 수 없다 —
    /// 헬퍼의 hostPids 정리는 controller.rs 의 BrowserProcessExited 가 한다.
    LogOnly,
}

fn failure_action(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> FailureAction {
    if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
        FailureAction::RecoverRenderer
    } else {
        FailureAction::LogOnly
    }
}

fn kind_name(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> &'static str {
    match kind {
        k if k == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "browser exited",
        k if k == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "renderer exited",
        k if k == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
            "renderer unresponsive"
        }
        k if k == COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => {
            "frame renderer exited"
        }
        _ => "other process failed",
    }
}

/// 다시 불러오기 제한기(managed state). 이벤트 콜백과 다음 열기의 재시도가 같은 제한을 쓴다.
#[derive(Default)]
struct ReloadGuard(std::sync::Mutex<ReloadLimiter>);

/// 기동 직후 죽는 렌더러를 끝없이 다시 불러오지 않게 한다(Electron 의 render-process-gone 도 같은 이유로 새 창을 즉시 만들지 않는다).
/// 최근 `WINDOW` 안에 `MAX_RELOADS` 번을 넘기면 거절한다.
#[derive(Default)]
struct ReloadLimiter {
    recent: Vec<Instant>,
}

impl ReloadLimiter {
    const WINDOW: Duration = Duration::from_secs(60);
    const MAX_RELOADS: usize = 3;

    fn allow(&mut self, now: Instant) -> bool {
        self.recent
            .retain(|t| now.saturating_duration_since(*t) < Self::WINDOW);
        if self.recent.len() >= Self::MAX_RELOADS {
            return false;
        }
        self.recent.push(now);
        true
    }
}

// ---------------------------------------------------------------------------
// COM
// ---------------------------------------------------------------------------

/// 미리보기 창의 WebView2 에 보호 기능을 붙인다. UI 스레드로 넘겨 실행하므로 setup 에서 불러도 된다.
pub fn install(app: &AppHandle, window: &WebviewWindow) {
    app.manage(ReloadGuard::default());
    let app = app.clone();
    let result = window.with_webview(move |webview| {
        // SAFETY: with_webview 의 클로저는 WebView2 가 만들어진 UI 스레드에서 실행된다.
        if let Err(e) = unsafe { attach(&webview.controller(), &app) } {
            eprintln!("[peekaboo] WebView2 보호 기능을 붙이지 못했다: {e}");
        }
    });
    if let Err(e) = result {
        eprintln!("[peekaboo] WebView2 에 접근하지 못했다(보호 기능): {e}");
    }
}

/// # Safety
/// WebView2 가 만들어진 UI 스레드에서만 부른다.
unsafe fn attach(
    controller: &ICoreWebView2Controller,
    app: &AppHandle,
) -> windows::core::Result<()> {
    let core = controller.CoreWebView2()?;
    let mut token = 0i64;

    // 프로세스 실패. 렌더러가 죽었을 때 복구하는 유일한 길이라 다른 보호 기능보다 먼저, 반드시 등록한다
    // (아래 항목들은 실패해도 로그만 남기고 계속한다).
    let failed_app = app.clone();
    let failed = ProcessFailedEventHandler::create(Box::new(move |sender, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        args.ProcessFailedKind(&mut kind)?;
        eprintln!(
            "[peekaboo] WebView2 {} ({})",
            kind_name(kind),
            failure_details(&args)
        );
        if failure_action(kind) == FailureAction::RecoverRenderer {
            recover_renderer(&failed_app, sender.as_ref());
        }
        Ok(())
    }));
    core.add_ProcessFailed(&failed, &mut token)?;

    // 브라우저 가속키 끄기
    match core
        .Settings()
        .and_then(|s| s.cast::<ICoreWebView2Settings3>())
        .and_then(|s| s.SetAreBrowserAcceleratorKeysEnabled(false))
    {
        Ok(()) => {}
        Err(e) => eprintln!("[peekaboo] 브라우저 가속키를 끄지 못했다: {e}"),
    }

    // 서브프레임 이동 차단. 메인 프레임 이동은 Tauri 의 on_navigation 이 막는다(preview.rs).
    let frames = NavigationStartingEventHandler::create(Box::new(|_, args| {
        if let Some(args) = args {
            let mut uri = PWSTR::null();
            args.Uri(&mut uri)?;
            let uri = take_pwstr(uri);
            if !allow_frame_navigation(&uri) {
                eprintln!("[peekaboo] 서브프레임 이동 차단: {uri}");
                args.SetCancel(true)?;
            }
        }
        Ok(())
    }));
    if let Err(e) = core.add_FrameNavigationStarting(&frames, &mut token) {
        eprintln!("[peekaboo] 서브프레임 이동 차단을 붙이지 못했다: {e}");
    }

    // 우클릭 복사
    let menu_app = app.clone();
    let menu = ContextMenuRequestedEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            // 메뉴는 항상 숨긴다
            args.SetHandled(true)?;
            let mut text = PWSTR::null();
            args.ContextMenuTarget()?.SelectionText(&mut text)?;
            let text = take_pwstr(text);
            preview::copy_to_clipboard(&menu_app, &text, "right-click");
        }
        Ok(())
    }));
    match core
        .cast::<ICoreWebView2_11>()
        .and_then(|c| c.add_ContextMenuRequested(&menu, &mut token))
    {
        Ok(()) => {}
        Err(e) => eprintln!("[peekaboo] 우클릭 복사를 붙이지 못했다: {e}"),
    }
    Ok(())
}

/// 로그에 남길 이유·종료 코드(ICoreWebView2ProcessFailedEventArgs2, 오래된 런타임에는 없다)
fn failure_details(
    args: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2ProcessFailedEventArgs,
) -> String {
    let Ok(args2) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() else {
        return "상세 정보 없음".into();
    };
    let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
    let mut exit_code = 0i32;
    // SAFETY: 이벤트 인자는 이 호출 동안 유효하다
    unsafe {
        let _ = args2.Reason(&mut reason);
        let _ = args2.ExitCode(&mut exit_code);
    }
    format!("reason={} exit={exit_code}", reason.0)
}

/// 렌더러가 죽었다. ready 를 내려 새 페이지가 다시 알릴 때까지 show 가 기다리게 하고, 열려 있던 미리보기는 닫고
/// (죽은 화면을 띄워 둘 이유가 없다. 다음에 Space 를 누르면 다시 열린다), 페이지를 다시 불러온다.
///
/// Electron 은 이때 창을 파기하고 다음 show 가 새 창을 만들었다(spare 창 교체도 있다). 여기서는 창을 계속 쓰고 reload 로 대체한다.
fn recover_renderer(app: &AppHandle, sender: Option<&ICoreWebView2>) {
    app.state::<PreviewState>().mark_not_ready();
    if let Some(controller) = app.try_state::<Arc<Controller>>() {
        controller.close();
    }
    let Some(core) = sender else {
        app.state::<PreviewState>().reload_refused();
        return;
    };
    try_reload(app, core);
}

/// 제한기를 확인하고 페이지를 다시 불러온다. 거절되거나 `Reload()` 가 실패하면 "다시 불러오기 필요" 를 남겨,
/// 다음 show 가 제한 창이 지난 뒤 다시 시도하게 한다(`PreviewState::begin_reload_retry`). 이 표시가 없으면 ready 가 영영 오지 않아
/// 렌더러가 죽은 뒤 앱을 다시 켜기 전까지 미리보기가 뜨지 않는다.
fn try_reload(app: &AppHandle, core: &ICoreWebView2) {
    let state = app.state::<PreviewState>();
    let allowed = app
        .state::<ReloadGuard>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .allow(Instant::now());
    if !allowed {
        eprintln!(
            "[peekaboo] 렌더러가 {}초 안에 {}번 넘게 죽어 지금은 다시 불러오지 않는다(다음 열기에서 다시 시도)",
            ReloadLimiter::WINDOW.as_secs(),
            ReloadLimiter::MAX_RELOADS
        );
        state.reload_refused();
        return;
    }
    // SAFETY: UI 스레드에서만 불린다(이벤트 콜백, 또는 with_webview 클로저).
    match unsafe { core.Reload() } {
        Ok(()) => {
            state.reload_started();
            eprintln!("[peekaboo] 렌더러 페이지를 다시 불러온다");
        }
        Err(e) => {
            state.reload_refused();
            eprintln!("[peekaboo] 페이지를 다시 불러오지 못했다: {e}");
        }
    }
}

/// 다음 열기가 ready 를 기다리기 전에, 앞선 재로드가 거절·실패했었다면 다시 시도한다. UI 스레드로 넘겨 실행한다.
pub fn retry_reload(app: &AppHandle) {
    let Some(window) = app.get_webview_window(preview::LABEL) else {
        return;
    };
    let handle = app.clone();
    let result = window.with_webview(move |webview| {
        // SAFETY: with_webview 의 클로저는 WebView2 가 만들어진 UI 스레드에서 실행된다.
        match unsafe { webview.controller().CoreWebView2() } {
            Ok(core) => try_reload(&handle, &core),
            Err(e) => {
                handle.state::<PreviewState>().reload_refused();
                eprintln!("[peekaboo] 다시 불러오기를 위해 WebView2 를 얻지 못했다: {e}");
            }
        }
    });
    if let Err(e) = result {
        app.state::<PreviewState>().reload_refused();
        eprintln!("[peekaboo] 다시 불러오기를 UI 스레드에 넘기지 못했다: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_만_서브프레임_이동을_허용한다() {
        assert!(allow_frame_navigation("about:srcdoc"));
        assert!(allow_frame_navigation("about:blank"));
        for bad in [
            "https://example.com/",
            "http://peek.localhost/x",
            "file:///C:/x.html",
            "javascript:alert(1)",
            "data:text/html,x",
            "blob:http://tauri.localhost/1",
            "ABOUT:blank",
            " about:blank",
            "",
        ] {
            assert!(!allow_frame_navigation(bad), "{bad}");
        }
    }

    #[test]
    fn 렌더러_종료만_복구하고_나머지는_로그만() {
        assert_eq!(
            failure_action(COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED),
            FailureAction::RecoverRenderer
        );
        for k in [
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        ] {
            assert_eq!(failure_action(k), FailureAction::LogOnly);
        }
    }

    #[test]
    fn 다시_불러오기는_제한_시간_안에_세_번까지() {
        let mut l = ReloadLimiter::default();
        let t0 = Instant::now();
        assert!(l.allow(t0));
        assert!(l.allow(t0 + Duration::from_secs(1)));
        assert!(l.allow(t0 + Duration::from_secs(2)));
        assert!(!l.allow(t0 + Duration::from_secs(3)));
        // 오래된 것이 창(60초)을 벗어나면 다시 허용: 61초 시점에 남는 것은 2초의 것 하나
        let t = t0 + Duration::from_secs(61);
        assert!(l.allow(t));
        assert!(l.allow(t));
        assert!(!l.allow(t));
    }
}
