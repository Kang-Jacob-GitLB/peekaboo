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

use tauri::{AppHandle, State, Webview};

use crate::capture;
use crate::files::{self, DirListing};
use crate::layout::ContentSize;
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

/// 닫기 버튼
#[tauri::command]
pub fn close(
    webview: Webview,
    app: AppHandle,
    state: State<'_, PreviewState>,
) -> Result<(), String> {
    from_preview(&webview)?;
    if state.is_open() {
        preview::hide(&app);
    }
    Ok(())
}

/// 텍스트 선택 여부. 지금은 저장만 한다(헬퍼가 Ctrl+C 를 가로챌지 정하는 데 쓰이는 것은 M3).
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
