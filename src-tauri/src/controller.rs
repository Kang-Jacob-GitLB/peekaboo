// 헬퍼와 미리보기 창을 잇는 호스트 본체. Electron 의 src/main/index.ts 에서 헬퍼 쪽 결선을 맡던 부분이다.
// 헬퍼 메시지를 session.rs 가 동작 목록으로 바꾸면 여기서 실제 창·헬퍼·네이티브 브리지에 적용한다.
//
// 스레드: 헬퍼 메시지는 헬퍼 감시 태스크(tokio)에서 도착한다. 거기서는 오래 막으면 안 되므로 파일 정보를 읽고 렌더러를
// 기다리는 `preview::show_in` 은 전용 스레드 하나(`show_worker`)가 도착 순서대로 처리한다. 그래서 선택이 연달아 바뀌어도
// 허용 경로가 뒤바뀌지 않는다. 닫기는 이 스레드를 거치지 않고 바로 상태를 바꾸며, 큐에서 기다리던 열기 요청은
// 닫기 세대(`PreviewState::epoch`)로 버린다.
//
// 헬퍼의 `state{previewOpen}`: 열기 요청을 만들 때 true 를 먼저 보내므로(session.rs), 표시가 끝나지 않는 모든 경로
// (닫기·큐에서 폐기·포기·표시 실패)에서 false 가 뒤따라야 한다. 경로마다 값을 정해 보내면 스레드 사이 순서가 엇갈릴 때
// 틀어지므로, 상태가 바뀔 때마다 `sync_helper_state` 가 PreviewState 를 읽어 그 값을 그대로 보낸다(순서를 잠금으로 직렬화).

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewWindow};
use webview2_com::BrowserProcessExitedEventHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller, ICoreWebView2Environment, ICoreWebView2Environment5,
};
use windows::core::Interface;

use crate::helper::{Helper, HelperMessage, HostMessage};
use crate::native::NativeBridge;
use crate::preview::{self, PreviewState};
use crate::session::{self, Action, Ctx, Session};

struct ShowRequest {
    path: String,
    selection_count: usize,
    /// 요청 시점의 닫기 세대
    epoch: u64,
}

/// 큐에 쌓인 열기 요청 중 마지막 것만 남긴다(`first` 다음으로 이미 도착해 있는 요청을 모두 소비).
/// 선택이 연달아 바뀌면(방향키 연타) 중간 파일은 파일 정보·렌더러 왕복만 낭비하고 화면에 잠깐 비칠 뿐이다.
///
/// 닫기와 섞여도 마지막 요청만 남기는 것이 같은 결과다. 닫기는 큐를 거치지 않고 닫기 세대(`epoch`)를 올리므로,
/// 요청 사이에 닫기가 있었다면 앞 요청의 세대는 이미 낡아 어차피 `begin_in` 에서 버려진다. 닫기가 없었다면 같은 세대의
/// 앞 요청은 뒤 요청이 허용 경로를 덮어쓰므로 버려도 달라지지 않는다. 즉 마지막 요청의 세대가 항상 가장 새롭다.
fn latest_request(first: ShowRequest, rx: &mpsc::Receiver<ShowRequest>) -> ShowRequest {
    let mut latest = first;
    while let Ok(next) = rx.try_recv() {
        latest = next;
    }
    latest
}

struct HostInfo {
    /// 단축키 설정. 설정 화면(M4)이 생기기 전에는 시작할 때 정한 값 그대로다.
    enabled: bool,
    /// WebView2 브라우저 프로세스. `<select>` 팝업 같은 창을 소유한다. 프로세스가 끝나면 None 으로 되돌린다.
    browser_pid: Option<u32>,
}

pub struct Controller {
    app: AppHandle,
    helper: Helper,
    session: Mutex<Session>,
    host: Mutex<HostInfo>,
    /// `sync_helper_state` 의 "상태 읽기 + 보내기"를 직렬화한다
    state_sync: Mutex<()>,
    show_tx: mpsc::Sender<ShowRequest>,
    /// 헬퍼가 한 번이라도 `ready` 를 보냈는지(디버그 단독 미리보기가 헬퍼를 기다리는 데 쓴다)
    helper_ready: (Mutex<bool>, Condvar),
}

impl Controller {
    pub fn new(app: AppHandle, helper: Helper, hotkey_enabled: bool) -> Arc<Self> {
        let (show_tx, show_rx) = mpsc::channel::<ShowRequest>();
        let worker_app = app.clone();
        std::thread::Builder::new()
            .name("preview-show".into())
            .spawn(move || {
                // Controller 가 사라지면(송신 쪽이 닫히면) 끝난다
                while let Ok(first) = show_rx.recv() {
                    let req = latest_request(first, &show_rx);
                    preview::show_in(&worker_app, &req.path, req.selection_count, Some(req.epoch));
                }
            })
            .expect("preview-show 스레드를 만들지 못했다");
        Arc::new(Controller {
            app,
            helper,
            session: Mutex::new(Session::default()),
            host: Mutex::new(HostInfo {
                enabled: hotkey_enabled,
                browser_pid: None,
            }),
            state_sync: Mutex::new(()),
            show_tx,
            helper_ready: (Mutex::new(false), Condvar::new()),
        })
    }

    /// WebView2 브라우저 프로세스 PID 를 얻고(UI 스레드에서만 얻을 수 있다) 헬퍼를 시작한다.
    /// `start_helper` 가 None 이면 헬퍼를 띄우지 않는다(디버그 단독 미리보기). Some(경로 오버라이드)면 띄운다.
    /// PID 를 못 얻어도 앱 PID 만으로 헬퍼를 띄운다.
    pub fn init(self: &Arc<Self>, window: &WebviewWindow, start_helper: Option<Option<PathBuf>>) {
        let (on_ui, on_error) = (Arc::clone(self), Arc::clone(self));
        let start_on_error = start_helper.clone();
        let result = window.with_webview(move |webview| {
            // SAFETY: with_webview 의 클로저는 WebView2 가 만들어진 UI 스레드에서 실행된다.
            let pid = unsafe { browser_process_id(&webview.controller()) };
            // 브라우저 프로세스가 끝나면(크래시·재시작) PID 가 다른 프로세스에 재사용될 수 있으므로 hostPids 에서 뺀다
            if let Err(e) = unsafe { watch_browser_exit(&webview.environment(), &on_ui) } {
                eprintln!("[peekaboo] WebView2 브라우저 종료 이벤트를 등록하지 못했다: {e}");
            }
            on_ui.finish_init(pid, start_helper);
        });
        if let Err(e) = result {
            eprintln!("[peekaboo] WebView2 에 접근하지 못했다: {e}");
            on_error.finish_init(None, start_on_error);
        }
    }

    fn finish_init(
        self: &Arc<Self>,
        browser_pid: Option<u32>,
        start_helper: Option<Option<PathBuf>>,
    ) {
        match browser_pid {
            Some(pid) => eprintln!("[peekaboo] WebView2 브라우저 프로세스 pid={pid}"),
            None => eprintln!("[peekaboo] WebView2 브라우저 프로세스 PID 를 얻지 못했다"),
        }
        self.host
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .browser_pid = browser_pid;
        if let Some(path_override) = start_helper {
            let this = Arc::clone(self);
            self.helper
                .start(path_override, move |msg| this.on_helper_message(msg));
        }
    }

    fn host_pids(&self) -> (bool, Vec<u32>) {
        let host = self.host.lock().unwrap_or_else(|e| e.into_inner());
        let mut pids = vec![std::process::id()];
        pids.extend(host.browser_pid);
        (host.enabled, pids)
    }

    /// WebView2 브라우저 프로세스가 끝났다. 알고 있던 PID 와 같으면 hostPids 에서 빼고 헬퍼에 설정을 다시 보낸다.
    fn on_browser_exited(&self, pid: u32) {
        {
            let mut host = self.host.lock().unwrap_or_else(|e| e.into_inner());
            if host.browser_pid != Some(pid) {
                return;
            }
            host.browser_pid = None;
        }
        eprintln!("[peekaboo] WebView2 브라우저 프로세스 종료 pid={pid}");
        let (enabled, host_pids) = self.host_pids();
        let ctx = Ctx {
            preview_open: false, // config 에는 쓰이지 않는다
            enabled,
            app_pid: std::process::id(),
            host_pids: &host_pids,
        };
        self.helper.send(&ctx.config());
    }

    /// 헬퍼에 현재의 `previewOpen`(PreviewState 의 열림·여는 중)을 알린다. 상태가 바뀔 수 있는 모든 곳
    /// (열기 시작, 닫기, 열기 포기·실패, 큐에서 폐기)에서 부르며 여러 번 불러도 같은 값이다.
    /// 읽기와 보내기를 한 잠금 안에서 해서, 여러 스레드가 겹쳐도 마지막으로 보낸 값이 마지막으로 읽은 상태와 같다.
    pub fn sync_helper_state(&self) {
        let _order = self.state_sync.lock().unwrap_or_else(|e| e.into_inner());
        let open = self.app.state::<PreviewState>().is_open();
        self.helper.send(&session::state_message(open));
    }

    /// 헬퍼의 첫 `ready` 를 기다린다. 제한 시간 안에 오면 true.
    pub fn wait_helper_ready(&self, timeout: Duration) -> bool {
        let (lock, cond) = &self.helper_ready;
        let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let (guard, _) = cond
            .wait_timeout_while(guard, timeout, |ready| !*ready)
            .unwrap_or_else(|e| e.into_inner());
        *guard
    }

    /// 헬퍼가 보낸 메시지를 처리한다. 헬퍼 감시 태스크에서 불리므로 오래 막지 않는다.
    pub fn on_helper_message(&self, msg: HelperMessage) {
        if matches!(msg, HelperMessage::Ready { .. }) {
            let (lock, cond) = &self.helper_ready;
            *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
            cond.notify_all();
        }
        let state = self.app.state::<PreviewState>();
        // 열기 요청이 큐에 들어가기 전에 닫히는 경우를 막으려고, 판단에 쓸 상태와 같은 시점의 세대를 잡아 둔다
        let epoch = state.epoch();
        let (enabled, host_pids) = self.host_pids();
        let ctx = Ctx {
            preview_open: state.is_open(),
            enabled,
            app_pid: std::process::id(),
            host_pids: &host_pids,
        };
        let actions = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .on_message(msg, &ctx);
        for action in actions {
            self.run(action, epoch);
        }
    }

    fn run(&self, action: Action, epoch: u64) {
        match action {
            Action::Send(msg) => {
                if matches!(msg, HostMessage::Config { .. }) {
                    eprintln!("[peekaboo] 헬퍼에 설정 전송: {}", msg.to_line().trim_end());
                }
                self.helper.send(&msg);
            }
            Action::Show {
                path,
                selection_count,
            } => {
                let _ = self.show_tx.send(ShowRequest {
                    path,
                    selection_count,
                    epoch,
                });
            }
            Action::Close => self.close(),
            Action::NativeResult {
                id,
                status,
                message,
            } => self
                .app
                .state::<NativeBridge>()
                .on_result(id, status, message),
            Action::NativeCloseAll(reason) => self.app.state::<NativeBridge>().close_all(reason),
            Action::Log(line) => eprintln!("[peekaboo] {line}"),
        }
    }

    /// 미리보기를 닫는다(헬퍼의 Esc·Space·Enter, 렌더러의 ✕ 가 모두 이 길을 탄다).
    /// 응답을 기다리던 열기 요청을 취소하고, 헬퍼에 알려 Esc 삼키기·선택 감시를 끈다.
    /// `hide` 가 열려 있지 않았다고 해도 알린다(멱등): 큐에서 기다리던 열기 때문에 헬퍼는 이미 true 를 받았을 수 있다.
    pub fn close(&self) {
        self.session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel_pending();
        preview::hide(&self.app);
        self.sync_helper_state();
    }
}

/// WebView2 브라우저 프로세스 종료 이벤트(`ICoreWebView2Environment5::add_BrowserProcessExited`)를 등록한다.
/// 이벤트는 UI 스레드에서 불린다. 등록은 앱이 사는 동안 유지하므로 토큰은 보관하지 않는다.
///
/// # Safety
/// WebView2 가 만들어진 UI 스레드에서만 부른다.
unsafe fn watch_browser_exit(
    env: &ICoreWebView2Environment,
    controller: &Arc<Controller>,
) -> windows::core::Result<()> {
    let env5: ICoreWebView2Environment5 = env.cast()?;
    let controller = Arc::clone(controller);
    let handler = BrowserProcessExitedEventHandler::create(Box::new(move |_, args| {
        let mut pid = 0u32;
        if let Some(args) = args {
            // SAFETY: 이벤트 인자는 이 호출 동안 유효하다
            if unsafe { args.BrowserProcessId(&mut pid) }.is_ok() && pid != 0 {
                controller.on_browser_exited(pid);
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    env5.add_BrowserProcessExited(&handler, &mut token)
}

/// # Safety
/// WebView2 가 만들어진 UI 스레드에서만 부른다.
unsafe fn browser_process_id(controller: &ICoreWebView2Controller) -> Option<u32> {
    let core = controller.CoreWebView2().ok()?;
    let mut pid = 0u32;
    core.BrowserProcessId(&mut pid).ok()?;
    (pid != 0).then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(path: &str, epoch: u64) -> ShowRequest {
        ShowRequest {
            path: path.to_owned(),
            selection_count: 1,
            epoch,
        }
    }

    #[test]
    fn 쌓인_열기_요청은_마지막_것만_남긴다() {
        let (tx, rx) = mpsc::channel();
        tx.send(req("b", 0)).unwrap();
        tx.send(req("c", 0)).unwrap();
        tx.send(req("d", 0)).unwrap();
        let got = latest_request(req("a", 0), &rx);
        assert_eq!((got.path.as_str(), got.epoch), ("d", 0));
        // 큐를 비웠으므로 다음 요청은 새로 기다린다
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn 요청이_하나면_그대로() {
        let (_tx, rx) = mpsc::channel();
        let got = latest_request(req("a", 3), &rx);
        assert_eq!((got.path.as_str(), got.epoch), ("a", 3));
    }

    /// 닫기가 사이에 끼면 뒤 요청의 세대가 더 새롭다. 합쳐도 그 세대 하나만 남아 앞 요청(낡은 세대)이 되살아나지 않는다.
    #[test]
    fn 닫기가_섞여도_가장_새_세대의_요청이_남는다() {
        let st = PreviewState::default();
        let (tx, rx) = mpsc::channel();
        tx.send(req("old", st.epoch())).unwrap();
        st.hide(); // 닫기: 큐를 거치지 않고 세대만 올린다
        tx.send(req("new", st.epoch())).unwrap();
        let first = rx.recv().unwrap();
        let got = latest_request(first, &rx);
        assert_eq!(got.path, "new");
        assert!(st.begin_in(&got.path, Some(got.epoch)).is_some());
        // 그 뒤 닫히면 합쳐진 요청도 낡은 세대라 시작하지 않는다
        st.hide();
        assert!(st.begin_in(&got.path, Some(got.epoch)).is_none());
    }
}
