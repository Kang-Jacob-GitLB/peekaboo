// 트레이 아이콘과 메뉴. Electron 의 src/main/tray.ts 와 같은 구성이다.
//   [Peekaboo <버전> — <상태>](비활성) / 구분선 / 스페이스바로 미리보기(체크) / Windows 시작 시 실행(체크) / 구분선
//   / 업데이트 항목(상태에 따라 문구가 바뀐다) / 로그 폴더 열기 / 종료
// 업데이트 항목은 업데이터(updater.rs)의 상태를 따른다. 상태가 바뀌면 메뉴를 새로 만든다(`refresh`).
//
// 메뉴의 항목 구성·체크 상태·툴팁은 순수 함수(`menu_entries`·`tooltip`·`action_for`)로 두어 화면 없이 시험한다.
// "로그 폴더 열기"는 JS 에 권한을 주지 않으려고 호스트가 `ShellExecuteW` 를 직접 부른다.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::path::BaseDirectory;
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::controller::Controller;
use crate::helper::HelperStatus;
use crate::settings::SettingsStore;
use crate::startup;
use crate::updater::{UpdateState, Updater};

const TRAY_ID: &str = "peekaboo-tray";
/// `bundle.resources` 가 설치 폴더에 놓는 파일 이름
const ICON_RESOURCE: &str = "tray.png";

const ID_TITLE: &str = "title";
const ID_HOTKEY: &str = "hotkey";
const ID_LAUNCH: &str = "launch";
const ID_UPDATE: &str = "update";
const ID_LOGS: &str = "logs";
const ID_QUIT: &str = "quit";

/// 헬퍼 상태의 표시 문구
fn status_text(status: HelperStatus) -> &'static str {
    match status {
        HelperStatus::Starting => "시작 중…",
        HelperStatus::Running => "동작 중",
        HelperStatus::Restarting => "헬퍼 재시작 중…",
        HelperStatus::Missing => "헬퍼 실행 파일 없음",
        HelperStatus::Stopped => "중지됨",
    }
}

/// 단축키가 꺼져 있으면 헬퍼 상태 대신 그것을 보인다
fn state_text(hotkey_enabled: bool, status: HelperStatus) -> &'static str {
    if hotkey_enabled {
        status_text(status)
    } else {
        "단축키 꺼짐"
    }
}

/// 업데이트 메뉴 항목의 문구와 활성 여부. Electron 의 tray.ts `updateItem` 과 같다.
/// `enabled`: 업데이트를 쓸 수 있는 빌드인가(꺼진 빌드는 "업데이트 확인" 이 비활성).
pub fn update_label(state: &UpdateState, enabled: bool) -> (String, bool) {
    match state {
        UpdateState::Checking => ("업데이트 확인 중…".into(), false),
        UpdateState::Downloading { version, percent } => (
            format!("업데이트 내려받는 중… ({version}, {percent}%)"),
            false,
        ),
        UpdateState::Ready { version } => (format!("다시 시작하여 업데이트 ({version})"), true),
        UpdateState::Idle | UpdateState::Latest | UpdateState::Error => {
            ("업데이트 확인".into(), enabled)
        }
    }
}

/// 메뉴·툴팁을 정하는 입력
#[derive(Debug, Clone, Copy)]
pub struct TrayState<'a> {
    pub version: &'a str,
    pub hotkey_enabled: bool,
    /// OS 의 시작 시 실행 등록 상태(`launch_available` 이 false 면 조회하지 않고 false)
    pub launch_at_startup: bool,
    /// 시작 시 실행을 바꿀 수 있는가. 디버그(개발) 실행은 개발용 exe 가 등록되므로 막는다.
    pub launch_available: bool,
    pub status: HelperStatus,
    pub update: &'a UpdateState,
    /// 업데이트를 쓸 수 있는 빌드인가
    pub update_enabled: bool,
}

pub fn tooltip(state: &TrayState) -> String {
    format!(
        "Peekaboo — {}",
        state_text(state.hotkey_enabled, state.status)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Separator,
    Item {
        id: &'static str,
        label: String,
        enabled: bool,
    },
    Check {
        id: &'static str,
        label: String,
        enabled: bool,
        checked: bool,
    },
}

pub fn menu_entries(state: &TrayState) -> Vec<Entry> {
    let text = state_text(state.hotkey_enabled, state.status);
    let (update_text, update_enabled) = update_label(state.update, state.update_enabled);
    vec![
        Entry::Item {
            id: ID_TITLE,
            label: format!("Peekaboo {} — {text}", state.version),
            enabled: false,
        },
        Entry::Separator,
        Entry::Check {
            id: ID_HOTKEY,
            label: "스페이스바로 미리보기".into(),
            enabled: true,
            checked: state.hotkey_enabled,
        },
        Entry::Check {
            id: ID_LAUNCH,
            label: "Windows 시작 시 실행".into(),
            enabled: state.launch_available,
            checked: state.launch_available && state.launch_at_startup,
        },
        Entry::Separator,
        Entry::Item {
            id: ID_UPDATE,
            label: update_text,
            enabled: update_enabled,
        },
        Entry::Item {
            id: ID_LOGS,
            label: "로그 폴더 열기".into(),
            enabled: true,
        },
        Entry::Item {
            id: ID_QUIT,
            label: "종료".into(),
            enabled: true,
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    ToggleHotkey,
    ToggleLaunch,
    Update,
    OpenLogs,
    Quit,
}

pub fn action_for(id: &str) -> Option<MenuAction> {
    match id {
        ID_HOTKEY => Some(MenuAction::ToggleHotkey),
        ID_LAUNCH => Some(MenuAction::ToggleLaunch),
        ID_UPDATE => Some(MenuAction::Update),
        ID_LOGS => Some(MenuAction::OpenLogs),
        ID_QUIT => Some(MenuAction::Quit),
        _ => None,
    }
}

/// 시작 시 실행을 바꿀 수 있는 실행인가(릴리스 빌드만)
pub const fn launch_available() -> bool {
    !cfg!(debug_assertions)
}

pub struct AppTray {
    app: AppHandle,
    tray: TrayIcon,
    status: Mutex<HelperStatus>,
}

impl AppTray {
    pub fn new(app: &AppHandle) -> tauri::Result<Arc<Self>> {
        let icon = load_icon(app);
        let mut builder = TrayIconBuilder::with_id(TRAY_ID)
            // Electron 처럼 우클릭에서만 메뉴를 연다
            .show_menu_on_left_click(false)
            .on_menu_event(on_menu_event);
        if let Some(icon) = icon {
            builder = builder.icon(icon);
        }
        let tray = builder.build(app)?;
        let this = Arc::new(AppTray {
            app: app.clone(),
            tray,
            status: Mutex::new(HelperStatus::Starting),
        });
        this.refresh();
        Ok(this)
    }

    /// 헬퍼 상태가 바뀌었다. 감시 스레드에서 불려도 되도록 메뉴 갱신은 메인 스레드에서 한다.
    pub fn set_status(self: &Arc<Self>, status: HelperStatus) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = status;
        let this = Arc::clone(self);
        if let Err(e) = self.app.run_on_main_thread(move || this.refresh()) {
            log_warn!("트레이 갱신을 메인 스레드에 넘기지 못했다: {e}");
        }
    }

    /// 업데이트 상태가 바뀌었다. 다른 스레드에서 불려도 되도록 메뉴 갱신은 메인 스레드에서 한다.
    pub fn refresh_later(self: &Arc<Self>) {
        let this = Arc::clone(self);
        if let Err(e) = self.app.run_on_main_thread(move || this.refresh()) {
            log_warn!("트레이 갱신을 메인 스레드에 넘기지 못했다: {e}");
        }
    }

    /// 메뉴와 툴팁을 현재 상태로 다시 만든다
    pub fn refresh(&self) {
        let settings = self.app.state::<SettingsStore>().get();
        let launch_available = launch_available();
        let launch_at_startup = launch_available
            && current_exe().is_some_and(|exe| {
                startup::is_enabled(startup::RUN_KEY, startup::VALUE_NAME, &exe)
            });
        let version = self.app.package_info().version.to_string();
        // 업데이터는 트레이보다 먼저 만들지만(main.rs), 없으면 꺼진 것으로 본다
        let (update, update_enabled) = match self.app.try_state::<Arc<Updater<tauri::Wry>>>() {
            Some(u) => (u.state(), u.enabled()),
            None => (UpdateState::Idle, false),
        };
        let state = TrayState {
            version: &version,
            hotkey_enabled: settings.hotkey_enabled,
            launch_at_startup,
            launch_available,
            status: *self.status.lock().unwrap_or_else(|e| e.into_inner()),
            update: &update,
            update_enabled,
        };
        if let Err(e) = self.tray.set_tooltip(Some(tooltip(&state))) {
            log_warn!("트레이 툴팁을 바꾸지 못했다: {e}");
        }
        match build_menu(&self.app, &menu_entries(&state)) {
            Ok(menu) => {
                if let Err(e) = self.tray.set_menu(Some(menu)) {
                    log_warn!("트레이 메뉴를 바꾸지 못했다: {e}");
                }
            }
            Err(e) => log_warn!("트레이 메뉴를 만들지 못했다: {e}"),
        }
    }
}

fn build_menu(app: &AppHandle, entries: &[Entry]) -> tauri::Result<Menu<tauri::Wry>> {
    enum Built {
        Separator(PredefinedMenuItem<tauri::Wry>),
        Item(MenuItem<tauri::Wry>),
        Check(CheckMenuItem<tauri::Wry>),
    }
    let mut built = Vec::with_capacity(entries.len());
    for entry in entries {
        built.push(match entry {
            Entry::Separator => Built::Separator(PredefinedMenuItem::separator(app)?),
            Entry::Item { id, label, enabled } => {
                Built::Item(MenuItem::with_id(app, *id, label, *enabled, None::<&str>)?)
            }
            Entry::Check {
                id,
                label,
                enabled,
                checked,
            } => Built::Check(CheckMenuItem::with_id(
                app,
                *id,
                label,
                *enabled,
                *checked,
                None::<&str>,
            )?),
        });
    }
    let items: Vec<&dyn IsMenuItem<tauri::Wry>> = built
        .iter()
        .map(|b| match b {
            Built::Separator(i) => i as &dyn IsMenuItem<tauri::Wry>,
            Built::Item(i) => i,
            Built::Check(i) => i,
        })
        .collect();
    Menu::with_items(app, &items)
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let Some(action) = action_for(&event.id.0) else {
        return;
    };
    let tray = app.state::<Arc<AppTray>>();
    match action {
        MenuAction::ToggleHotkey => {
            let enabled = !app.state::<SettingsStore>().get().hotkey_enabled;
            app.state::<SettingsStore>().set_hotkey_enabled(enabled);
            app.state::<Arc<Controller>>().set_hotkey_enabled(enabled);
            tray.refresh();
        }
        MenuAction::ToggleLaunch => {
            if launch_available() {
                if let Some(exe) = current_exe() {
                    let enabled = !startup::is_enabled(startup::RUN_KEY, startup::VALUE_NAME, &exe);
                    if let Err(e) = startup::set_enabled(
                        startup::RUN_KEY,
                        startup::STARTUP_APPROVED_KEY,
                        startup::VALUE_NAME,
                        &exe,
                        enabled,
                    ) {
                        log_error!("시작 시 실행을 바꾸지 못했다: {e}");
                    }
                }
            }
            tray.refresh();
        }
        MenuAction::Update => {
            if let Some(updater) = app.try_state::<Arc<Updater<tauri::Wry>>>() {
                // 받아 둔 것이 있으면 바로 설치(성공하면 돌아오지 않는다), 아니면 확인
                if updater.is_ready() {
                    if !updater.install(true) {
                        tray.refresh();
                    }
                } else {
                    updater.check_manually();
                }
            }
        }
        MenuAction::OpenLogs => {
            if let Some(dir) = crate::appdata::log_dir() {
                open_folder(&dir);
            }
        }
        MenuAction::Quit => {
            // 받아 둔 업데이트가 있으면 설치하며 끝낸다(설치 뒤 재실행하지 않는다). 성공하면 돌아오지 않는다.
            if let Some(updater) = app.try_state::<Arc<Updater<tauri::Wry>>>() {
                if updater.is_ready() && updater.install(false) {
                    return;
                }
            }
            app.exit(0)
        }
    }
}

fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// 트레이 아이콘: 번들 리소스의 `tray.png`, 못 읽으면 앱 기본 아이콘
fn load_icon(app: &AppHandle) -> Option<Image<'static>> {
    let from_resource = app
        .path()
        .resolve(ICON_RESOURCE, BaseDirectory::Resource)
        .ok()
        .and_then(|p| Image::from_path(p).ok());
    if from_resource.is_none() {
        log_warn!("트레이 아이콘({ICON_RESOURCE})을 읽지 못해 기본 아이콘을 쓴다");
    }
    from_resource.or_else(|| app.default_window_icon().map(|i| i.clone().to_owned()))
}

/// 폴더를 탐색기로 연다(없으면 만든다)
fn open_folder(dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        log_warn!("로그 폴더를 만들지 못했다: {e}");
    }
    // 같은 이름의 파일이 있으면 ShellExecute 가 그 파일을 실행해 버리므로 폴더일 때만 연다
    if !dir.is_dir() {
        log_warn!("로그 폴더가 폴더가 아니라 열지 않는다: {}", dir.display());
        return;
    }
    let wide: Vec<u16> = dir
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: wide 는 NUL 로 끝나는 UTF-16 경로, 나머지 인자는 NULL 허용
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 32 이하면 실패 코드(ShellExecute 의 반환 규약)
    if result.0 as isize <= 32 {
        log_warn!(
            "로그 폴더를 열지 못했다: ShellExecuteW={}",
            result.0 as isize
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(
        hotkey: bool,
        launch: bool,
        available: bool,
        status: HelperStatus,
    ) -> TrayState<'static> {
        static IDLE: UpdateState = UpdateState::Idle;
        TrayState {
            version: "1.2.3",
            hotkey_enabled: hotkey,
            launch_at_startup: launch,
            launch_available: available,
            status,
            update: &IDLE,
            update_enabled: true,
        }
    }

    fn item<'a>(entries: &'a [Entry], want: &str) -> &'a Entry {
        entries
            .iter()
            .find(|e| match e {
                Entry::Item { id, .. } | Entry::Check { id, .. } => *id == want,
                Entry::Separator => false,
            })
            .unwrap()
    }

    #[test]
    fn 메뉴_구성과_순서는_electron_과_같다() {
        let e = menu_entries(&state(true, false, true, HelperStatus::Running));
        let kinds: Vec<&str> = e
            .iter()
            .map(|e| match e {
                Entry::Separator => "-",
                Entry::Item { id, .. } | Entry::Check { id, .. } => id,
            })
            .collect();
        assert_eq!(
            kinds,
            ["title", "-", "hotkey", "launch", "-", "update", "logs", "quit"]
        );
        assert_eq!(
            item(&e, "title"),
            &Entry::Item {
                id: "title",
                label: "Peekaboo 1.2.3 — 동작 중".into(),
                enabled: false
            }
        );
        assert_eq!(
            item(&e, "hotkey"),
            &Entry::Check {
                id: "hotkey",
                label: "스페이스바로 미리보기".into(),
                enabled: true,
                checked: true
            }
        );
    }

    #[test]
    fn 단축키가_꺼지면_상태_문구가_바뀐다() {
        let off = state(false, false, true, HelperStatus::Running);
        assert_eq!(tooltip(&off), "Peekaboo — 단축키 꺼짐");
        let e = menu_entries(&off);
        assert!(
            matches!(item(&e, "title"), Entry::Item { label, .. } if label == "Peekaboo 1.2.3 — 단축키 꺼짐")
        );
        assert!(matches!(
            item(&e, "hotkey"),
            Entry::Check { checked: false, .. }
        ));
    }

    #[test]
    fn 헬퍼_상태_문구() {
        let t = |s| tooltip(&state(true, false, true, s));
        assert_eq!(t(HelperStatus::Starting), "Peekaboo — 시작 중…");
        assert_eq!(t(HelperStatus::Running), "Peekaboo — 동작 중");
        assert_eq!(t(HelperStatus::Restarting), "Peekaboo — 헬퍼 재시작 중…");
        assert_eq!(t(HelperStatus::Missing), "Peekaboo — 헬퍼 실행 파일 없음");
        assert_eq!(t(HelperStatus::Stopped), "Peekaboo — 중지됨");
    }

    #[test]
    fn 시작_시_실행_체크는_os_상태를_따른다() {
        let e = menu_entries(&state(true, true, true, HelperStatus::Running));
        assert!(matches!(
            item(&e, "launch"),
            Entry::Check {
                enabled: true,
                checked: true,
                ..
            }
        ));
        let e = menu_entries(&state(true, false, true, HelperStatus::Running));
        assert!(matches!(
            item(&e, "launch"),
            Entry::Check {
                enabled: true,
                checked: false,
                ..
            }
        ));
    }

    /// 개발(디버그) 실행: 항목은 비활성이고, 설령 등록돼 있다는 값이 들어와도 체크하지 않는다
    #[test]
    fn 개발_실행에서는_시작_시_실행이_비활성이다() {
        let e = menu_entries(&state(true, true, false, HelperStatus::Running));
        assert!(matches!(
            item(&e, "launch"),
            Entry::Check {
                enabled: false,
                checked: false,
                ..
            }
        ));
        // 이 빌드(시험은 디버그)에서도 같은 판정이다
        assert_eq!(launch_available(), !cfg!(debug_assertions));
    }

    #[test]
    fn 메뉴_id_는_동작에_대응한다() {
        assert_eq!(action_for("hotkey"), Some(MenuAction::ToggleHotkey));
        assert_eq!(action_for("launch"), Some(MenuAction::ToggleLaunch));
        assert_eq!(action_for("update"), Some(MenuAction::Update));
        assert_eq!(action_for("logs"), Some(MenuAction::OpenLogs));
        assert_eq!(action_for("quit"), Some(MenuAction::Quit));
        assert_eq!(action_for("title"), None);
        assert_eq!(action_for("x"), None);
    }

    #[test]
    fn 업데이트_항목_문구는_상태를_따른다() {
        let v = |s: &str| s.to_string();
        assert_eq!(
            update_label(&UpdateState::Idle, true),
            (v("업데이트 확인"), true)
        );
        // 업데이트를 쓸 수 없는 빌드는 "업데이트 확인" 이 비활성이다
        assert_eq!(
            update_label(&UpdateState::Idle, false),
            (v("업데이트 확인"), false)
        );
        assert_eq!(
            update_label(&UpdateState::Latest, true),
            (v("업데이트 확인"), true)
        );
        assert_eq!(
            update_label(&UpdateState::Error, true),
            (v("업데이트 확인"), true)
        );
        assert_eq!(
            update_label(&UpdateState::Checking, true),
            (v("업데이트 확인 중…"), false)
        );
        assert_eq!(
            update_label(
                &UpdateState::Downloading {
                    version: v("1.2.0"),
                    percent: 40
                },
                true
            ),
            (v("업데이트 내려받는 중… (1.2.0, 40%)"), false)
        );
        assert_eq!(
            update_label(
                &UpdateState::Ready {
                    version: v("1.2.0")
                },
                true
            ),
            (v("다시 시작하여 업데이트 (1.2.0)"), true)
        );
    }

    #[test]
    fn 받아_둔_업데이트는_메뉴에서_바로_설치할_수_있다() {
        let ready = UpdateState::Ready {
            version: "1.2.0".into(),
        };
        let mut s = state(true, false, true, HelperStatus::Running);
        s.update = &ready;
        let e = menu_entries(&s);
        assert_eq!(
            item(&e, "update"),
            &Entry::Item {
                id: "update",
                label: "다시 시작하여 업데이트 (1.2.0)".into(),
                enabled: true
            }
        );
    }
}
