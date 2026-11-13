// Peekaboo Tauri 호스트(전환 계획 docs/tauri-migration.md M2·M3a).
// 미리보기 창·peek 프로토콜·렌더러 커맨드에 더해 C# 헬퍼 감시, 키 흐름, 네이티브 미리보기를 연결한다.
// 트레이, 단일 인스턴스, 설정, 로그 파일은 M4. 창 동작(복사·포커스 허용 등)의 나머지는 M3 의 뒷 작업.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("Peekaboo 는 Windows 전용이다");

mod capture;
mod commands;
mod controller;
mod files;
mod helper;
mod layout;
mod native;
mod peek;
mod preview;
mod session;

use std::path::PathBuf;
use std::time::Duration;

use tauri::{Manager, RunEvent};

/// 개발 전용 단독 미리보기: `PEEKABOO_PREVIEW=<파일> [PEEKABOO_CAPTURE=<png>]`.
/// 헬퍼·키 입력 없이 미리보기 창만 확인한다. 캡처 모드는 WebView2 데이터 폴더를 임시 폴더로 써서
/// 설치된 Peekaboo 와 겹치지 않는다.
struct DevPreview {
    path: String,
    capture: Option<PathBuf>,
    data_dir: Option<PathBuf>,
}

impl DevPreview {
    fn from_env() -> Option<Self> {
        if !cfg!(debug_assertions) {
            return None;
        }
        let path = std::env::var("PEEKABOO_PREVIEW")
            .ok()
            .filter(|p| !p.is_empty())?;
        let capture = std::env::var_os("PEEKABOO_CAPTURE").map(PathBuf::from);
        let data_dir = capture
            .is_some()
            .then(|| std::env::temp_dir().join(format!("peekaboo-capture-{}", std::process::id())));
        Some(DevPreview {
            path,
            capture,
            data_dir,
        })
    }
}

/// 헬퍼를 띄울지와 그 경로 오버라이드(디버그 빌드 전용 환경 변수).
/// - `PEEKABOO_HELPER=1`: 기본 경로의 헬퍼를 띄운다. 그 밖의 비어 있지 않은 값은 헬퍼 실행 파일 경로로 보고 그것을 띄운다.
/// - 단독 미리보기(`PEEKABOO_PREVIEW`)에서는 위 변수가 있을 때만 띄운다(설치된 Peekaboo 의 키보드 훅과 겹치지 않게).
///   일반 실행(상주)에서는 변수가 없어도 띄운다.
///
/// 반환값: None = 띄우지 않음, Some(None) = 기본 경로, Some(Some(경로)) = 오버라이드.
fn helper_plan(dev_preview: bool) -> Option<Option<PathBuf>> {
    let var = if cfg!(debug_assertions) {
        std::env::var("PEEKABOO_HELPER")
            .ok()
            .filter(|v| !v.is_empty())
    } else {
        None
    };
    match var {
        Some(v) if v != "1" => Some(Some(PathBuf::from(v))),
        Some(_) => Some(None),
        None if dev_preview => None,
        None => Some(None),
    }
}

/// 단축키 설정의 시작값. 디버그 빌드에서 `PEEKABOO_HOTKEY=0` 이면 꺼서 시작한다(헬퍼가 키를 가로채지 않으므로,
/// 설치된 Peekaboo 가 실행 중일 때 두 훅이 같은 Space 를 두고 다투지 않고 실측할 수 있다).
fn hotkey_enabled() -> bool {
    !(cfg!(debug_assertions) && std::env::var("PEEKABOO_HOTKEY").is_ok_and(|v| v == "0"))
}

fn main() {
    let dev = DevPreview::from_env();
    let start_helper = helper_plan(dev.is_some());
    let enabled = hotkey_enabled();
    let data_dir = dev.as_ref().and_then(|d| d.data_dir.clone());
    let cleanup_dir = data_dir.clone();

    let helper = helper::Helper::default();
    let native_helper = helper.clone();
    let native = native::NativeBridge::new(move |msg| native_helper.send(msg));
    let exit_helper = helper.clone();

    let app = tauri::Builder::default()
        .manage(native)
        .manage(preview::PreviewState::default())
        .manage(capture::CaptureConfig(
            dev.as_ref().and_then(|d| d.capture.clone()),
        ))
        .register_asynchronous_uri_scheme_protocol("peek", peek::handle)
        .invoke_handler(tauri::generate_handler![
            commands::ready,
            commands::fit,
            commands::rendered,
            commands::close,
            commands::text_selected,
            commands::list_dir,
            commands::native_open,
            commands::native_resize,
            commands::native_close,
        ])
        .setup(move |app| {
            // setup 은 동기 커맨드·이벤트 핸들러가 아니라 앱 초기화 단계라 창 생성이 안전하다.
            let window = preview::create_window(app.handle(), data_dir)?;
            let controller = controller::Controller::new(app.handle().clone(), helper, enabled);
            app.manage(controller.clone());
            // WebView2 브라우저 PID 를 얻은 뒤 헬퍼를 띄운다(hostPids 에 담기 위해)
            let with_helper = start_helper.is_some();
            controller.init(&window, start_helper);
            if let Some(dev) = dev {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    // 네이티브 미리보기는 헬퍼가 떠 있어야 하므로, 헬퍼를 띄운 경우 첫 ready 를 먼저 기다린다
                    if with_helper {
                        controller.wait_helper_ready(Duration::from_secs(10));
                    }
                    preview::show(&handle, &dev.path)
                });
                if let Some(out) = dev.capture {
                    // 렌더 완료 신호가 끝내 오지 않는 경우를 위한 안전장치
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(capture::RENDER_TIMEOUT);
                        capture::finish(&handle, Err("렌더 완료 신호가 오지 않았다".into()), &out);
                    });
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Tauri 앱 생성 실패");

    app.run(move |_app, event| match event {
        // 미리보기 창은 숨겨질 뿐 닫히지 않지만, 창이 모두 닫혀도 상주하도록 종료 요청을 막아 둔다
        // (`exit()` 로 요청한 종료는 code 가 있어 통과한다)
        RunEvent::ExitRequested { api, code, .. } if code.is_none() => api.prevent_exit(),
        RunEvent::Exit => {
            // 헬퍼에 정상 종료를 요청한다. 이 앱이 이 경로 없이 죽어도 stdin 이 닫혀 헬퍼가 스스로 끝난다.
            exit_helper.stop();
            if let Some(dir) = &cleanup_dir {
                // WebView2 가 아직 파일을 잡고 있으면 지우지 못한다. 임시 폴더라 남아도 해가 없다.
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        _ => {}
    });
}
