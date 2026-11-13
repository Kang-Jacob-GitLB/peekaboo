// 미리보기 창. Electron 의 src/main/preview-window.ts 에 대응한다.
// 상태(PreviewState)는 Tauri 와 무관한 순수 로직이라 cargo test 로 검증하고, 창 생성·배치·표시는 아래 함수들이 맡는다.
//
// 범위: 창 생성, show 페이로드 전송, 첫 fit 대기, 크기 맞춤, 표시·숨김, 닫을 때 네이티브 미리보기 정리.
// 아직 없음(M3 의 뒷 작업): spare 창 교체, 헤더 드래그(userMoved), 클릭 시 포커스 허용(setActivatable), 복사.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

use crate::files;
use crate::layout::{self, ContentSize};

/// 미리보기 창의 label. capabilities/preview.json 의 windows 와 peek.rs 의 허용 검사가 같은 값을 본다.
pub const LABEL: &str = "preview";

/// 렌더러(src/renderer/host/tauri.ts 의 Events)가 듣는 이벤트
pub const EVENT_SHOW: &str = "preview:show";
pub const EVENT_SHOW_ERROR: &str = "preview:show-error";
pub const EVENT_HIDE: &str = "preview:hide";

/// 처음 띄울 때 렌더러가 내용 크기를 알려 주기를 기다리는 최대 시간(기본 크기로 떴다가 바뀌며 튀지 않게)
const FIRST_FIT_WAIT: Duration = Duration::from_millis(150);
/// 렌더러가 리스너 등록을 마치고 ready 를 보내기를 기다리는 최대 시간.
/// Electron 은 로드를 무기한 기다리지만(preview-window.ts), 여기서는 상한을 둔다. 렌더러가 끝내 ready 를 안 보내면
/// show 스레드가 영원히 `opening` 인 채로 남아 사용자의 다음 입력(다른 파일 요청·닫기)까지 막히기 때문이다.
/// 시간이 지나면 `opening` 을 되돌리고 이번 요청만 포기한다(다음 요청은 다시 기다린다).
const READY_WAIT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// 상태
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Inner {
    /// 표시 순번. 늦게 도착한 이전 요청의 결과를 버리는 데 쓴다.
    seq: u64,
    /// 닫기(`hide`)마다 올라가는 세대. 헬퍼 메시지로 열기를 요청한 시점의 세대를 들고 있다가, 그 사이 닫혔으면
    /// `begin_in` 이 요청을 버린다(요청이 큐에서 기다리는 동안 닫혀도 뒤늦게 뜨지 않게). seq 와 같은 잠금 안에 있다.
    epoch: u64,
    /// 지금 미리보기 중인 파일 경로(창 라벨은 LABEL 하나). seq 와 같은 Mutex 에 있어 함께 바뀐다.
    allowed: Option<String>,
    /// 렌더러가 리스너 등록을 마쳤는지
    ready: bool,
    visible: bool,
    /// 창을 띄우기 전 파일 정보·렌더러 로드를 기다리는 중
    opening: bool,
    /// 창을 띄우기 전에 받은 크기 요청(None = 아직 없음, Some(None) = 기본 크기)
    first_fit: Option<Option<ContentSize>>,
    text_selected: bool,
    /// 마지막 show 요청의 시작 시각(`rendered` 지연 로그용)
    started: Option<(u64, Instant)>,
}

/// `fit` 커맨드를 어떻게 처리할지
#[derive(Debug, PartialEq)]
pub enum FitRoute {
    /// 오래된 seq. 버린다.
    Stale,
    /// 아직 띄우기 전: show 가 이 크기로 띄운다.
    Stored,
    /// 이미 떠 있음: 지금 창 크기를 바꾼다.
    Apply(Option<ContentSize>),
}

/// `finish_show` 의 결과
#[derive(Debug, PartialEq)]
pub enum Finish {
    /// 그 사이 취소·교체됐다.
    Cancelled,
    Done,
    /// 띄우는 동안 더 새로운 fit 이 도착했다. 지금 적용해야 한다.
    Late(Option<ContentSize>),
}

#[derive(Default)]
pub struct PreviewState {
    inner: Mutex<Inner>,
    cond: Condvar,
}

impl PreviewState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // 다른 스레드가 패닉해도 상태 자체는 단순 값이라 그대로 쓴다
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 세대를 따지지 않는 `begin_in`(테스트용 줄임). 허용 경로를 `path` 로 바꾼다. (seq, 이미 떠 있었는지)
    #[cfg(test)]
    pub fn begin(&self, path: &str) -> (u64, bool) {
        self.begin_in(path, None)
            .expect("세대를 지정하지 않으면 항상 시작한다")
    }

    /// 지금의 닫기 세대
    pub fn epoch(&self) -> u64 {
        self.lock().epoch
    }

    /// 새 표시 요청을 시작한다. 허용 경로를 `path` 로 바꾼다. (seq, 이미 떠 있었는지)
    /// `epoch` 가 주어졌고 그 뒤에 닫혔으면 시작하지 않고 None.
    pub fn begin_in(&self, path: &str, epoch: Option<u64>) -> Option<(u64, bool)> {
        let mut s = self.lock();
        if epoch.is_some_and(|e| e != s.epoch) {
            return None;
        }
        s.seq += 1;
        s.allowed = Some(path.to_owned());
        s.opening = true;
        s.started = Some((s.seq, Instant::now()));
        // 떠 있을 때의 fit 은 저장하지 않고 바로 적용하므로(route_fit) 이 값은 처음 띄우는 요청에서만 쓰인다
        s.first_fit = None;
        Some((s.seq, s.visible))
    }

    pub fn is_current(&self, seq: u64) -> bool {
        self.lock().seq == seq
    }

    /// isAllowedPath 와 같은 의미: 지금 미리보기 중인 (창 라벨, 경로)와 바이트 단위로 정확히 일치할 때만 허용한다
    /// (대소문자 무시·정규화·슬래시 보정 없음). peek 프로토콜과 list_dir 이 쓴다.
    pub fn is_allowed(&self, label: &str, path: &str) -> bool {
        match &self.lock().allowed {
            Some(p) => label.as_bytes() == LABEL.as_bytes() && p.as_bytes() == path.as_bytes(),
            None => false,
        }
    }

    /// 띄우기를 끝내지 못하고 포기했다(창 없음·ready 시간 초과·표시 실패). 아직 현재 요청이면 `opening` 을 되돌린다.
    pub fn abort_open(&self, seq: u64) {
        let mut s = self.lock();
        if s.seq == seq {
            s.opening = false;
        }
    }

    /// 창을 띄우지 못했다. 아직 현재 요청이면 보이는 중이 아닌 것으로 되돌린다.
    pub fn fail_show(&self, seq: u64) {
        let mut s = self.lock();
        if s.seq == seq {
            s.visible = false;
            s.opening = false;
        }
    }

    /// 보이는 중이거나 띄우는 중
    pub fn is_open(&self) -> bool {
        let s = self.lock();
        s.visible || s.opening
    }

    pub fn mark_ready(&self) {
        self.lock().ready = true;
        self.cond.notify_all();
    }

    /// ready 를 기다린다. 제한 시간 안에 오면 true.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let guard = self.lock();
        let (guard, _) = self
            .cond
            .wait_timeout_while(guard, timeout, |s| !s.ready)
            .unwrap_or_else(|e| e.into_inner());
        guard.ready
    }

    /// 처음 띄우기 전 크기 요청을 기다린다. 요청이 없으면 None(기본 크기), 그 사이 취소됐으면 None.
    pub fn wait_first_fit(&self, seq: u64, timeout: Duration) -> Option<ContentSize> {
        let guard = self.lock();
        let (guard, _) = self
            .cond
            .wait_timeout_while(guard, timeout, |s| s.seq == seq && s.first_fit.is_none())
            .unwrap_or_else(|e| e.into_inner());
        if guard.seq != seq {
            return None;
        }
        guard.first_fit.flatten()
    }

    pub fn route_fit(&self, seq: u64, size: Option<ContentSize>) -> FitRoute {
        let mut s = self.lock();
        if s.seq != seq {
            return FitRoute::Stale;
        }
        if s.visible {
            return FitRoute::Apply(size);
        }
        s.first_fit = Some(size);
        self.cond.notify_all();
        FitRoute::Stored
    }

    /// 창을 띄운 뒤 호출한다. `applied` 는 띄울 때 적용한 크기(없으면 기본 크기).
    /// 첫 fit 대기가 끝난 뒤 visible 이 서기 전에 도착한 fit 은 저장만 되고 적용되지 않으므로, 달라졌으면 돌려준다.
    pub fn finish_show(&self, seq: u64, applied: Option<ContentSize>) -> Finish {
        let mut s = self.lock();
        if s.seq != seq {
            return Finish::Cancelled;
        }
        s.visible = true;
        s.opening = false;
        match s.first_fit {
            Some(latest) if latest != applied => Finish::Late(latest),
            _ => Finish::Done,
        }
    }

    /// 닫는다. 띄우는 중이면 취소한다(seq 를 올리면 대기 중인 show 가 창을 띄우지 않고 끝난다). 열려 있었으면 true.
    pub fn hide(&self) -> bool {
        let mut s = self.lock();
        s.seq += 1;
        s.epoch += 1;
        let was_open = s.visible || s.opening;
        s.visible = false;
        s.opening = false;
        s.allowed = None;
        s.text_selected = false;
        s.first_fit = None;
        self.cond.notify_all();
        was_open
    }

    /// 보이는 창에서만 기록한다(띄우기 전·닫은 뒤의 알림은 무시).
    pub fn set_text_selected(&self, selected: bool) {
        let mut s = self.lock();
        if s.visible {
            s.text_selected = selected;
        }
    }

    /// `rendered` 도착 시 지연(ms). 현재 요청의 seq 일 때만.
    pub fn since_start(&self, seq: u64) -> Option<u128> {
        match self.lock().started {
            Some((s, t0)) if s == seq => Some(t0.elapsed().as_millis()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 창
// ---------------------------------------------------------------------------

/// 렌더러가 이동할 수 있는 출처: 앱 자신(Windows 에서 wry 가 `tauri://localhost` 로 알려 준다)뿐이다.
/// 미리보기 중인 문서의 링크가 메인 프레임을 다른 곳으로 옮기지 못하게 한다.
fn is_app_url(url: &tauri::Url) -> bool {
    let host = url.host_str();
    match url.scheme() {
        "tauri" => host == Some("localhost"),
        "http" | "https" => host == Some("tauri.localhost"),
        _ => false,
    }
}

/// 다크 모드에서 흰 화면이 번쩍이지 않도록 렌더러 CSS 와 같은 배경
fn background(dark: bool) -> Color {
    if dark {
        Color(0x20, 0x20, 0x20, 0xff)
    } else {
        Color(0xf3, 0xf3, 0xf3, 0xff)
    }
}

/// 미리보기 창을 숨긴 채 만든다. Windows 에서는 동기 커맨드·이벤트 핸들러 안에서 창을 만들면 교착하므로
/// setup 이나 별도 스레드·async 태스크에서만 부른다.
pub fn create_window(
    app: &AppHandle,
    data_dir: Option<std::path::PathBuf>,
) -> tauri::Result<WebviewWindow> {
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Peekaboo")
        .inner_size(960.0, 720.0)
        .decorations(false)
        .always_on_top(true)
        .focusable(false)
        .skip_taskbar(true)
        .visible(false)
        .shadow(false)
        .minimizable(false)
        .maximizable(false)
        .background_color(background(false))
        // wry 는 핸들러가 없으면 새 창 요청을 거부하지만(wry 0.57.2 webview2/mod.rs:862-864, 핸들러가 없으면 SetHandled(true)) 명시해 둔다
        .on_new_window(|url, _features| {
            eprintln!("[peekaboo] 새 창 차단: {url}");
            NewWindowResponse::Deny
        })
        .on_navigation(|url| {
            let ok = is_app_url(url);
            if !ok {
                eprintln!("[peekaboo] 메인 프레임 이동 차단: {url}");
            }
            ok
        });
    if let Some(dir) = data_dir {
        builder = builder.data_directory(dir);
    }
    let window = builder.build()?;
    if window.theme().is_ok_and(|t| t == tauri::Theme::Dark) {
        window.set_background_color(Some(background(true)))?;
    }
    Ok(window)
}

/// 창을 `size` 에 맞춰 배치한다. 작업 영역은 monitor 의 것(물리 픽셀)이고, 크기 계산은 DIP 로 한 뒤 환산한다.
fn place(
    window: &WebviewWindow,
    monitor: &Monitor,
    size: Option<&ContentSize>,
) -> tauri::Result<()> {
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let (area_w, area_h) = (f64::from(area.size.width), f64::from(area.size.height));
    let (w, h) = match size {
        Some(size) => layout::fit_size(area_w / scale, area_h / scale, size),
        None => layout::default_size(area_w / scale, area_h / scale),
    };
    let width = (w * scale).round().min(area_w) as u32;
    let height = (h * scale).round().min(area_h) as u32;
    // 작업 영역 중앙. 창이 작업 영역보다 크면 왼쪽 위에 맞춘다.
    let x = area.position.x + ((area_w - f64::from(width)) / 2.0).max(0.0).round() as i32;
    let y = area.position.y + ((area_h - f64::from(height)) / 2.0).max(0.0).round() as i32;
    // 위치를 먼저 옮긴다. 배율이 다른 모니터로 옮길 때 set_size 를 먼저 하면 이어지는 이동에서 WM_DPICHANGED 가
    // 크기를 다시 환산해 방금 정한 크기가 틀어진다. 이동으로 배율이 바뀐 뒤에 크기를 정하면 결과가 그대로 남는다.
    // HWND 에 SetWindowPos 를 한 번 부르는 방법은 같은 재환산을 피하지 못하고 tao 를 우회하므로 쓰지 않았다.
    window.set_position(PhysicalPosition::new(x, y))?;
    window.set_size(PhysicalSize::new(width, height))
}

/// 처음 띄울 때: 커서가 있는 모니터
fn cursor_monitor(app: &AppHandle) -> Option<Monitor> {
    let pos = app.cursor_position().ok()?;
    app.monitor_from_point(pos.x, pos.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// 이미 떠 있는 창의 크기를 바꿀 때(`fit` 커맨드): 창이 있는 모니터(파일을 넘길 때 다른 모니터로 옮겨 가지 않게)
pub fn apply_fit(app: &AppHandle, size: Option<ContentSize>) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| cursor_monitor(app));
    if let Some(monitor) = monitor {
        if let Err(e) = place(&window, &monitor, size.as_ref()) {
            eprintln!("[peekaboo] 창 크기 맞춤 실패: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// 표시·숨김
// ---------------------------------------------------------------------------

/// 헬퍼에 현재 열림 상태를 알린다(controller.rs 의 `sync_helper_state`). 컨트롤러가 없는 환경(단독 미리보기 이전)에서는 아무 일도 없다.
fn sync_helper(app: &AppHandle) {
    if let Some(controller) = app.try_state::<std::sync::Arc<crate::controller::Controller>>() {
        controller.sync_helper_state();
    }
}

/// 띄우기를 포기한다. 헬퍼에는 열기 시작 때 previewOpen=true 를 보냈으므로 상태를 다시 알린다.
fn abort_open(app: &AppHandle, seq: u64) {
    app.state::<PreviewState>().abort_open(seq);
    sync_helper(app);
}

/// 창을 띄우지 못했다. `abort_open` 과 같이 헬퍼에 상태를 다시 알린다.
fn fail_show(app: &AppHandle, seq: u64) {
    app.state::<PreviewState>().fail_show(seq);
    sync_helper(app);
}

/// 파일 하나를 미리보기로 띄운다. 파일 I/O 와 대기가 있으므로 UI 스레드가 아닌 곳에서 부른다.
/// 마지막 단계(재확인 → 표시 → finish_show)는 `hide` 의 창 숨김과 같은 메인 스레드에서 직렬로 돈다(present).
pub fn show(app: &AppHandle, path: &str) {
    show_in(app, path, 1, None);
}

/// `show` 와 같다. `epoch` 가 주어졌고 그 뒤에 `hide` 됐으면 아무것도 하지 않는다. `selection_count` 는 탐색기에서 선택된 항목 수.
pub fn show_in(app: &AppHandle, path: &str, selection_count: usize, epoch: Option<u64>) {
    let state = app.state::<PreviewState>();
    let Some((seq, was_visible)) = state.begin_in(path, epoch) else {
        // 큐에서 기다리는 동안 닫혔다. 요청을 만들 때 헬퍼에 보낸 previewOpen=true 를 되돌린다
        // (닫은 쪽이 알렸어도 그 뒤에 도착한 true 가 있을 수 있다).
        sync_helper(app);
        return;
    };
    // 열기를 시작했음을 헬퍼에 알린다(요청을 만들 때 보낸 true 가 그 사이의 닫기 알림에 덮였을 수 있다)
    sync_helper(app);

    if app.get_webview_window(LABEL).is_none() {
        eprintln!("[peekaboo] 미리보기 창이 없다");
        abort_open(app, seq);
        return;
    }
    if !state.wait_ready(READY_WAIT) {
        eprintln!(
            "[peekaboo] 렌더러가 {}초 안에 준비되지 않았다",
            READY_WAIT.as_secs()
        );
        abort_open(app, seq);
        return;
    }
    // 렌더러 로드를 기다리는 동안 다른 파일이 요청됐거나 hide() 로 취소됐으면 여기서 끝낸다.
    let info = files::file_info(path);
    if !state.is_current(seq) {
        return;
    }
    let sent = match &info {
        Ok(file) => app.emit_to(
            LABEL,
            EVENT_SHOW,
            serde_json::json!({ "seq": seq, "file": file, "selectionCount": selection_count }),
        ),
        Err(e) => app.emit_to(
            LABEL,
            EVENT_SHOW_ERROR,
            serde_json::json!({ "seq": seq, "path": path, "message": e.to_string() }),
        ),
    };
    if let Err(e) = sent {
        eprintln!("[peekaboo] 표시 이벤트 전송 실패: {e}");
    }

    let mut applied = None;
    if !was_visible {
        // 기다리는 동안에도 opening 이 유지돼 그 사이 hide() 로 취소할 수 있다
        applied = state.wait_first_fit(seq, FIRST_FIT_WAIT);
        if !state.is_current(seq) {
            return;
        }
    }
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || present(&handle, seq, was_visible, applied)) {
        eprintln!("[peekaboo] 표시를 메인 스레드에 넘기지 못했다: {e}");
        abort_open(app, seq);
    }
}

/// show 의 마지막 단계. 메인 스레드에서 실행된다. hide 의 창 숨김도 메인 스레드에서 실행되므로,
/// 두 동작은 서로의 중간에 끼어들지 못한다(재확인 뒤 hide 가 상태를 바꾸면 그 hide 의 숨김이 뒤따라 실행된다).
fn present(app: &AppHandle, seq: u64, was_visible: bool, applied: Option<ContentSize>) {
    let state = app.state::<PreviewState>();
    if !state.is_current(seq) {
        return;
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        eprintln!("[peekaboo] 미리보기 창이 없다");
        abort_open(app, seq);
        return;
    };
    if !was_visible {
        if let Some(monitor) = cursor_monitor(app) {
            if let Err(e) = place(&window, &monitor, applied.as_ref()) {
                eprintln!("[peekaboo] 창 배치 실패: {e}");
            }
        }
        if let Err(e) = window.show() {
            eprintln!("[peekaboo] 창 표시 실패: {e}");
            fail_show(app, seq);
            return;
        }
        eprintln!(
            "[peekaboo] shown in {}ms",
            state.since_start(seq).unwrap_or_default()
        );
    }
    match state.finish_show(seq, applied) {
        Finish::Late(size) => apply_fit(app, size),
        Finish::Cancelled => {
            // 창을 띄운 직후 hide 가 상태를 바꿨다. 열려 있다고 볼 요청이 없으면 창을 숨긴다
            // (더 새로운 show 가 이어서 띄우는 중이면 그쪽이 맡는다).
            if !was_visible && !state.is_open() {
                let _ = window.hide();
            }
        }
        Finish::Done => {}
    }
}

/// 미리보기를 닫는다(렌더러의 ✕, 헬퍼의 Esc·Space). 열려 있었으면 true, 아니면 아무것도 하지 않고 false.
/// 닫기 세대는 열려 있었는지와 무관하게 올라가 큐에서 기다리는 열기 요청을 막는다.
/// 상태는 호출한 스레드에서 바로 바꾸고, 창 숨김은 메인 스레드에서 실행한다(show 의 present 와 직렬화).
pub fn hide(app: &AppHandle) -> bool {
    let was_open = app.state::<PreviewState>().hide();
    if !was_open {
        return false;
    }
    // 열려 있던 네이티브 미리보기 창은 렌더러가 닫기 전에 헬퍼에서 내린다
    app.state::<crate::native::NativeBridge>()
        .close_all("hidden");
    if let Err(e) = app.emit_to(LABEL, EVENT_HIDE, ()) {
        eprintln!("[peekaboo] hide 이벤트 전송 실패: {e}");
    }
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(LABEL) {
            if let Err(e) = window.hide() {
                eprintln!("[peekaboo] 창 숨김 실패: {e}");
            }
        }
    });
    if let Err(e) = queued {
        eprintln!("[peekaboo] 창 숨김을 메인 스레드에 넘기지 못했다: {e}");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn size(w: f64) -> ContentSize {
        ContentSize {
            width: w,
            height: w,
            grow: false,
        }
    }

    #[test]
    fn 앱_url_만_메인_프레임_이동을_허용한다() {
        for ok in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/index.html",
            "https://tauri.localhost/",
        ] {
            assert!(is_app_url(&ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "https://example.com/",
            "http://peek.localhost/x",
            "file:///C:/x.html",
            "about:blank",
            "http://tauri.localhost.evil.com/",
            "javascript:alert(1)",
            "tauri://evil/",
        ] {
            assert!(!is_app_url(&bad.parse().unwrap()), "{bad}");
        }
    }

    #[test]
    fn 오래된_seq_의_fit_은_버린다() {
        let st = PreviewState::default();
        let (a, _) = st.begin("p");
        let (b, _) = st.begin("p");
        assert_eq!(st.route_fit(a, Some(size(10.0))), FitRoute::Stale);
        assert_eq!(st.route_fit(b, Some(size(10.0))), FitRoute::Stored);
    }

    #[test]
    fn 띄우기_전_fit_은_저장되고_뜬_뒤_fit_은_바로_적용된다() {
        let st = PreviewState::default();
        let (seq, was_visible) = st.begin("p");
        assert!(!was_visible);
        assert_eq!(st.route_fit(seq, Some(size(10.0))), FitRoute::Stored);
        assert_eq!(
            st.wait_first_fit(seq, Duration::from_millis(10)),
            Some(size(10.0))
        );
        assert_eq!(st.finish_show(seq, Some(size(10.0))), Finish::Done);
        assert_eq!(st.route_fit(seq, None), FitRoute::Apply(None));
        // 떠 있는 상태에서 다른 파일: 기다리지 않는다
        let (_, was_visible) = st.begin("p");
        assert!(was_visible);
    }

    #[test]
    fn 첫_fit_대기가_끝난_뒤_띄우기_전에_온_fit_은_finish_show_가_돌려준다() {
        let st = PreviewState::default();
        let (seq, _) = st.begin("p");
        st.route_fit(seq, Some(size(10.0)));
        let applied = st.wait_first_fit(seq, Duration::from_millis(10));
        // 대기가 끝난 뒤, 아직 visible 이 아닐 때 새 fit 이 온다
        assert_eq!(st.route_fit(seq, Some(size(20.0))), FitRoute::Stored);
        assert_eq!(st.finish_show(seq, applied), Finish::Late(Some(size(20.0))));
        // 이제 visible 이므로 이후 fit 은 바로 적용된다
        assert_eq!(st.route_fit(seq, None), FitRoute::Apply(None));
    }

    #[test]
    fn 늦은_fit_이_없으면_finish_show_는_done() {
        let st = PreviewState::default();
        let (seq, _) = st.begin("p");
        // 기본 크기(None)로 띄웠고 그 뒤 fit(null)도 같은 값이면 다시 적용하지 않는다
        st.route_fit(seq, None);
        assert_eq!(st.finish_show(seq, None), Finish::Done);
        let (seq, _) = st.begin("p");
        assert_eq!(st.finish_show(seq, None), Finish::Done);
    }

    #[test]
    fn 첫_fit_이_없으면_제한_시간_뒤_기본_크기() {
        let st = PreviewState::default();
        let (seq, _) = st.begin("p");
        assert_eq!(st.wait_first_fit(seq, Duration::from_millis(10)), None);
        // null(기본 크기) 요청도 같은 결과
        st.route_fit(seq, None);
        assert_eq!(st.wait_first_fit(seq, Duration::from_millis(10)), None);
    }

    #[test]
    fn 첫_fit_대기는_fit_이_오면_깨어난다() {
        let st = Arc::new(PreviewState::default());
        let (seq, _) = st.begin("p");
        let other = Arc::clone(&st);
        let t = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            other.route_fit(seq, Some(size(7.0)));
        });
        assert_eq!(
            st.wait_first_fit(seq, Duration::from_secs(5)),
            Some(size(7.0))
        );
        t.join().unwrap();
    }

    #[test]
    fn 띄우는_중_hide_하면_취소된다() {
        let st = PreviewState::default();
        let (seq, _) = st.begin("p");
        assert!(st.is_open());
        assert!(st.hide());
        assert!(!st.is_current(seq));
        assert_eq!(st.finish_show(seq, None), Finish::Cancelled);
        assert!(!st.is_open());
        // 닫혀 있을 때 hide 는 열려 있던 게 아니다
        assert!(!st.hide());
    }

    #[test]
    fn 허용_경로는_정확히_일치할_때만() {
        let s = PreviewState::default();
        assert!(!s.is_allowed(LABEL, r"C:\a\b.txt"), "비어 있으면 거부");
        s.begin(r"C:\a\b.txt");
        assert!(s.is_allowed(LABEL, r"C:\a\b.txt"));
        assert!(!s.is_allowed(LABEL, r"C:\A\b.txt"), "대소문자");
        assert!(!s.is_allowed(LABEL, r"c:\a\b.txt"), "드라이브 대소문자");
        assert!(!s.is_allowed(LABEL, r"C:\a\b.txt\"), "뒤 슬래시");
        assert!(!s.is_allowed(LABEL, r"C:/a/b.txt"), "구분자");
        assert!(!s.is_allowed(LABEL, r"C:\a\b.txt "), "뒤 공백");
        assert!(!s.is_allowed(LABEL, ""));
        assert!(!s.is_allowed("main", r"C:\a\b.txt"), "다른 라벨");
        assert!(!s.is_allowed("Preview", r"C:\a\b.txt"), "라벨 대소문자");
    }

    #[test]
    fn 허용_nfc_nfd_는_다른_경로() {
        let s = PreviewState::default();
        s.begin("C:\\a\\\u{00E9}.txt");
        assert!(!s.is_allowed(LABEL, "C:\\a\\e\u{0301}.txt"));
    }

    #[test]
    fn 허용_경로는_표시_순번과_함께_교체되고_닫으면_지워진다() {
        let s = PreviewState::default();
        s.begin("a");
        s.begin("b");
        assert!(!s.is_allowed(LABEL, "a"));
        assert!(s.is_allowed(LABEL, "b"));
        s.hide();
        assert!(!s.is_allowed(LABEL, "b"));
    }

    #[test]
    fn 띄우기를_포기하면_현재_요청만_opening_을_되돌린다() {
        let st = PreviewState::default();
        let (a, _) = st.begin("p");
        let (b, _) = st.begin("q");
        st.abort_open(a);
        assert!(st.is_open(), "오래된 요청의 포기는 무시");
        st.abort_open(b);
        assert!(!st.is_open());
        // 표시 실패는 visible 도 되돌린다
        let (c, _) = st.begin("r");
        assert_eq!(st.finish_show(c, None), Finish::Done);
        assert!(st.is_open());
        st.fail_show(c);
        assert!(!st.is_open());
    }

    #[test]
    fn 요청_뒤_닫히면_그_세대의_열기는_시작하지_않는다() {
        let st = PreviewState::default();
        let e = st.epoch();
        // 큐에서 기다리는 동안 닫기
        st.hide();
        assert_eq!(st.begin_in("p", Some(e)), None);
        assert!(!st.is_open());
        assert!(!st.is_allowed(LABEL, "p"));
        // 새 세대의 요청과 세대 미지정 요청은 시작한다
        assert!(st.begin_in("p", Some(st.epoch())).is_some());
        assert!(st.begin_in("q", None).is_some());
        // 열기끼리는 세대가 올라가지 않아 앞 요청이 뒤 요청을 막지 않는다
        let e = st.epoch();
        assert!(st.begin_in("a", Some(e)).is_some());
        assert!(st.begin_in("b", Some(e)).is_some());
        assert!(st.is_allowed(LABEL, "b"));
    }

    #[test]
    fn ready_를_기다린다() {
        let st = Arc::new(PreviewState::default());
        assert!(!st.wait_ready(Duration::from_millis(10)));
        let other = Arc::clone(&st);
        let t = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            other.mark_ready();
        });
        assert!(st.wait_ready(Duration::from_secs(5)));
        t.join().unwrap();
    }

    #[test]
    fn 텍스트_선택은_보이는_창에서만_기록한다() {
        let st = PreviewState::default();
        st.set_text_selected(true);
        assert!(!st.lock().text_selected);
        let (seq, _) = st.begin("p");
        st.finish_show(seq, None);
        st.set_text_selected(true);
        assert!(st.lock().text_selected);
        st.hide();
        assert!(!st.lock().text_selected);
    }
}
