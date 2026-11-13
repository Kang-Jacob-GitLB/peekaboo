use tauri_build::{AppManifest, Attributes};

/// 웹뷰가 부를 수 있는 앱 커맨드. 여기에 등록하면 기본 "모든 창 허용"이 아니라 ACL(capabilities/preview.json)
/// 아래에 놓인다. 권한 식별자는 `allow-<커맨드 이름의 _ 를 - 로>` 이다.
const COMMANDS: &[&str] = &[
    "ready",
    "fit",
    "rendered",
    "close",
    "text_selected",
    "list_dir",
    "native_open",
    "native_resize",
    "native_close",
];

fn main() {
    tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(COMMANDS)))
        .expect("tauri-build 실패");
}
