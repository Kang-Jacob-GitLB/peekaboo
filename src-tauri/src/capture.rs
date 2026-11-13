// 개발용 캡처: `PEEKABOO_PREVIEW=<파일> PEEKABOO_CAPTURE=<png>` 로 띄우면 렌더 완료 뒤 미리보기 창의 웹 콘텐츠를
// WebView2 `ICoreWebView2::CapturePreview` 로 PNG 에 저장하고 종료한다(Electron 의 capturePage 에 해당).
// 전체 화면이 아니라 웹뷰만 찍는다. 헬퍼가 띄우는 네이티브 미리보기 창은 찍히지 않는다.

use std::cell::RefCell;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewWindow};
use webview2_com::CapturePreviewCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2Controller, COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::System::Com::{IStream, STGC_DEFAULT, STGM_CREATE, STGM_WRITE};
use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;

use crate::preview::{self, PreviewState};

/// `rendered` 뒤 화면이 그려질 때까지의 여유(이미지·PDF 는 `rendered` 직후에야 그려지는 것이 있다)
const SETTLE: Duration = Duration::from_millis(600);
/// 이 시간 안에 `rendered` 가 오지 않으면 실패로 끝낸다
pub const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

/// 캡처 모드 설정. 일반 실행에서는 `None`.
#[derive(Default)]
pub struct CaptureConfig(pub Option<PathBuf>);

type Done = Box<dyn FnOnce(Result<(), String>)>;

/// 렌더러의 `rendered` 신호를 받았을 때. 캡처 모드이고 현재 요청의 seq 일 때만 찍는다.
pub fn on_rendered(app: &AppHandle, seq: u64) {
    let Some(out) = app.state::<CaptureConfig>().0.clone() else {
        return;
    };
    if !app.state::<PreviewState>().is_current(seq) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SETTLE);
        let Some(window) = app.get_webview_window(preview::LABEL) else {
            return finish(&app, Err("미리보기 창이 없다".into()), &out);
        };
        let (app2, out2) = (app.clone(), out.clone());
        let started = capture_png(&window, out.clone(), move |result| {
            finish(&app2, result, &out2)
        });
        if let Err(e) = started {
            finish(&app, Err(e), &out);
        }
    });
}

/// 결과를 남기고 종료한다. 성공은 0, 실패는 1.
pub fn finish(app: &AppHandle, result: Result<(), String>, out: &Path) {
    match result {
        Ok(()) => {
            eprintln!("[peekaboo] captured {}", out.display());
            app.exit(0);
        }
        Err(e) => {
            eprintln!("[peekaboo] 캡처 실패: {e}");
            app.exit(1);
        }
    }
}

/// UI 스레드에서 CapturePreview 를 시작한다. 완료(성공·실패)는 UI 스레드에서 `done` 으로 알린다.
/// 기다리지 않는다: UI 스레드를 막으면 완료 콜백이 오지 않는다.
fn capture_png(
    window: &WebviewWindow,
    out: PathBuf,
    done: impl FnOnce(Result<(), String>) + Send + 'static,
) -> Result<(), String> {
    window
        .with_webview(move |webview| {
            // with_webview 의 클로저는 UI 스레드에서 실행된다. ICoreWebView2 호출은 그 스레드에서만 해야 한다.
            let done: Done = Box::new(done);
            // SAFETY: UI 스레드에서 부른다.
            unsafe { start(&webview.controller(), &out, done) }
        })
        .map_err(|e| e.to_string())
}

/// # Safety
/// WebView2 가 만들어진 UI 스레드에서만 부른다.
unsafe fn start(controller: &ICoreWebView2Controller, out: &Path, done: Done) {
    let (core, stream) = match open(controller, out) {
        Ok(v) => v,
        Err(e) => return done(Err(e)),
    };
    // 완료 콜백과 시작 실패 경로 중 먼저 오는 쪽이 한 번만 부른다.
    let done = Rc::new(RefCell::new(Some(done)));
    let (done_in_handler, stream_in_handler) = (Rc::clone(&done), stream.clone());
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        // 스트림을 flush 하고 놓은 뒤에 알린다(파일이 닫혀야 읽을 수 있다)
        let committed = stream_in_handler.Commit(STGC_DEFAULT);
        drop(stream_in_handler);
        if let Some(done) = done_in_handler.borrow_mut().take() {
            done(
                result
                    .map_err(|e| e.to_string())
                    .and(committed.map_err(|e| e.to_string())),
            );
        }
        Ok(())
    }));
    if let Err(e) = core.CapturePreview(
        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
        &stream,
        &handler,
    ) {
        if let Some(done) = done.borrow_mut().take() {
            done(Err(format!("CapturePreview: {e}")));
        }
    }
}

/// CoreWebView2 와, PNG 를 받을 파일 스트림을 얻는다.
unsafe fn open(
    controller: &ICoreWebView2Controller,
    out: &Path,
) -> Result<
    (
        webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
        IStream,
    ),
    String,
> {
    let core = controller
        .CoreWebView2()
        .map_err(|e| format!("CoreWebView2: {e}"))?;
    let abs = std::path::absolute(out).map_err(|e| e.to_string())?;
    let wide: Vec<u16> = abs
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let stream = SHCreateStreamOnFileEx(
        PCWSTR(wide.as_ptr()),
        STGM_CREATE.0 | STGM_WRITE.0,
        FILE_ATTRIBUTE_NORMAL.0,
        true,
        None::<&IStream>,
    )
    .map_err(|e| format!("SHCreateStreamOnFileEx: {e}"))?;
    Ok((core, stream))
}
