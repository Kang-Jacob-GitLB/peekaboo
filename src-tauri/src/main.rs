// Peekaboo Tauri 호스트(전환 계획 docs/tauri-migration.md M2: 앱 골격).
// 지금은 미리보기 창·peek 프로토콜·렌더러 커맨드까지다. 헬퍼 연결, 트레이, 단일 인스턴스, 설정, 로그 파일은 M3·M4.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("Peekaboo 는 Windows 전용이다");

mod capture;
mod commands;
mod files;
mod layout;
mod peek;
mod preview;

use std::path::PathBuf;

use tauri::RunEvent;

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

fn main() {
    let dev = DevPreview::from_env();
    if dev.is_none() {
        // 헬퍼 연결(M3)과 트레이(M4)가 없어 띄울 방법이 없다
        eprintln!(
            "[peekaboo] M2 골격: PEEKABOO_PREVIEW=<파일> 로 미리보기를 확인한다(디버그 빌드 전용)"
        );
        std::process::exit(2);
    }
    let data_dir = dev.as_ref().and_then(|d| d.data_dir.clone());
    let cleanup_dir = data_dir.clone();

    let app = tauri::Builder::default()
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
        ])
        .setup(move |app| {
            // setup 은 동기 커맨드·이벤트 핸들러가 아니라 앱 초기화 단계라 창 생성이 안전하다.
            preview::create_window(app.handle(), data_dir)?;
            if let Some(dev) = dev {
                let handle = app.handle().clone();
                std::thread::spawn(move || preview::show(&handle, &dev.path));
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

    app.run(move |_app, event| {
        if let RunEvent::Exit = event {
            if let Some(dir) = &cleanup_dir {
                // WebView2 가 아직 파일을 잡고 있으면 지우지 못한다. 임시 폴더라 남아도 해가 없다.
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    });
}
