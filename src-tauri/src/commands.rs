// 렌더러(src/renderer/host/tauri.ts)가 부르는 커맨드. Electron 의 preview-window.ts registerIpc 에 대응한다.
//
// 보안: 커맨드 이름은 build.rs 의 AppManifest 로 ACL 아래에 두고, capabilities/preview.json 이 preview 창에만 허용한다.
// 그 위에 커맨드마다 (1) 호출한 웹뷰가 미리보기 창인지, (2) 경로를 받으면 지금 미리보기 중인 경로와 정확히 일치하는지
// (preview.rs PreviewState::is_allowed, Electron 의 isAllowedPath 와 같은 의미) 다시 검사한다.
//
// 동기/async: 창을 만들지 않는 커맨드(ready·fit·rendered·close·text_selected)는 동기 커맨드다. 동기 커맨드는
// 메인(UI) 스레드에서 도착 순서대로 실행되므로, 같은 렌더러가 보낸 fit·close 가 뒤바뀌지 않는다.
// tauri 2.12.5 의 교착 경고(webview/webview_window.rs 의 WebviewWindowBuilder::new 문서: "On Windows, this function
// deadlocks when used in a synchronous command and event handlers")는 **창·웹뷰를 만드는** 호출에만 해당한다.
// 이 커맨드들이 부르는 창 API(set_position·set_size·current_monitor·run_on_main_thread)는 메인 스레드에서 부르면
// tauri-runtime-wry 의 send_user_message 가 큐를 거치지 않고 그 자리에서 처리하므로 재진입 교착이 없다.
// 파일 I/O 가 있는 list_dir 만 async 로 두고 blocking 풀에서 읽는다.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State, Webview};

use crate::capture;
use crate::controller::Controller;
use crate::files::{self, DirListing};
use crate::helper::{NativeStatus, PixelRect};
use crate::layout::ContentSize;
use crate::native::{NativeBridge, NativePreviewResult};
use crate::preview::{self, FitRoute, PreviewState};

const FORBIDDEN: &str = "Forbidden";

fn from_preview(webview: &Webview) -> Result<(), String> {
    if webview.label() == preview::LABEL {
        Ok(())
    } else {
        Err(FORBIDDEN.into())
    }
}

/// 렌더러가 리스너 등록을 마쳤다. 이 뒤에야 show 이벤트를 보낸다(host/tauri.ts 참고).
#[tauri::command]
pub fn ready(webview: Webview, state: State<'_, PreviewState>) -> Result<(), String> {
    from_preview(&webview)?;
    state.mark_ready();
    Ok(())
}

/// 창 크기를 내용에 맞춘다. size 가 null 이면 기본 크기.
#[tauri::command]
pub fn fit(
    webview: Webview,
    app: AppHandle,
    state: State<'_, PreviewState>,
    seq: u64,
    size: Option<ContentSize>,
) -> Result<(), String> {
    from_preview(&webview)?;
    if size.as_ref().is_some_and(|s| !s.is_valid()) {
        return Ok(());
    }
    if let FitRoute::Apply(size) = state.route_fit(seq, size) {
        preview::apply_fit(&app, size);
    }
    Ok(())
}

/// 렌더 완료 신호(표시 지연 로그, 캡처 모드의 촬영 시점)
#[tauri::command]
pub fn rendered(
    webview: Webview,
    app: AppHandle,
    state: State<'_, PreviewState>,
    seq: u64,
) -> Result<(), String> {
    from_preview(&webview)?;
    if let Some(ms) = state.since_start(seq) {
        eprintln!("[peekaboo] rendered seq={seq} in {ms}ms since start");
    }
    capture::on_rendered(&app, seq);
    Ok(())
}

/// 닫기 버튼. 헬퍼의 Esc·Space 와 같은 길(Controller::close)로 닫는다.
#[tauri::command]
pub fn close(
    webview: Webview,
    controller: State<'_, Arc<Controller>>,
    state: State<'_, PreviewState>,
) -> Result<(), String> {
    from_preview(&webview)?;
    if state.is_open() {
        controller.close();
    }
    Ok(())
}

/// 텍스트 선택 여부. 지금은 저장만 하고 헬퍼에는 알리지 않는다: 알리면 헬퍼가 Ctrl+C 를 삼키는데, 선택한 텍스트를
/// 클립보드에 쓰는 복사(`key{copy}` 처리)가 아직 없어 아무 일도 일어나지 않기 때문이다. 복사와 함께 연결한다.
#[tauri::command]
pub fn text_selected(
    webview: Webview,
    state: State<'_, PreviewState>,
    selected: bool,
) -> Result<(), String> {
    from_preview(&webview)?;
    state.set_text_selected(selected);
    Ok(())
}

/// 현재 표시 중인 폴더의 항목(최대 max 개)
#[tauri::command]
pub async fn list_dir(
    webview: Webview,
    state: State<'_, PreviewState>,
    path: String,
    max: u32,
) -> Result<DirListing, String> {
    from_preview(&webview)?;
    if !state.is_allowed(webview.label(), &path) {
        return Err(FORBIDDEN.into());
    }
    tauri::async_runtime::spawn_blocking(move || files::list_dir(&path, max))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Windows 미리보기 핸들러(Office 등)를 미리보기 창 안 rect(클라이언트 영역, 물리 픽셀)에 띄운다.
/// 현재 표시 중인 경로만 허용한다. 인터넷에서 받은 파일(Mark of the Web)은 외부 파서를 돌리지 않고 `unavailable` 로 답한다.
/// 헬퍼가 8초 안에 답하지 않으면 `failed` 로 끝난다(렌더러가 자체 렌더러로 넘어간다).
#[tauri::command]
pub async fn native_open(
    webview: Webview,
    app: AppHandle,
    state: State<'_, PreviewState>,
    native: State<'_, NativeBridge>,
    path: String,
    rect: PixelRect,
) -> Result<NativePreviewResult, String> {
    from_preview(&webview)?;
    if !state.is_allowed(webview.label(), &path) || !rect.is_valid() {
        return Err(FORBIDDEN.into());
    }
    let zone_path = path.clone();
    let from_internet =
        tauri::async_runtime::spawn_blocking(move || files::is_from_internet(&zone_path))
            .await
            .map_err(|e| e.to_string())?;
    if from_internet {
        return Ok(NativePreviewResult {
            handle: 0,
            status: NativeStatus::Unavailable,
            message: Some("internet-zone".into()),
        });
    }
    let window = app
        .get_webview_window(preview::LABEL)
        .ok_or_else(|| FORBIDDEN.to_string())?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize as i64;
    let dark = window.theme().is_ok_and(|t| t == tauri::Theme::Dark);
    let result = native.open(&path, hwnd, rect, dark).await;
    // 기다리는 사이 미리보기가 닫히거나 다른 파일로 바뀌었으면 띄운 핸들러 창을 바로 내린다
    if result.status == NativeStatus::Shown && !state.is_allowed(preview::LABEL, &path) {
        native.close(result.handle);
        return Ok(NativePreviewResult::failed(result.handle, "hidden"));
    }
    Ok(result)
}

/// 네이티브 미리보기 표시 영역 변경. 경로를 받지 않으므로 호출한 웹뷰와 handle(마지막으로 연 요청)만 검사한다.
#[tauri::command]
pub fn native_resize(
    webview: Webview,
    native: State<'_, NativeBridge>,
    handle: i32,
    rect: PixelRect,
) -> Result<(), String> {
    from_preview(&webview)?;
    if rect.is_valid() {
        native.resize(handle, rect);
    }
    Ok(())
}

/// 네이티브 미리보기 닫기(검사는 native_resize 와 같다)
#[tauri::command]
pub fn native_close(
    webview: Webview,
    native: State<'_, NativeBridge>,
    handle: i32,
) -> Result<(), String> {
    from_preview(&webview)?;
    native.close(handle);
    Ok(())
}
