// 옛 Electron 설치본이 남아 있는지 읽기 전용으로 확인하고, 있으면 로그에 수동 제거 안내를 남긴다.
// Tauri 설치기(src-tauri/windows/hooks.nsh)는 현재 사용자 설치본(HKCU)만 지운다. "모든 사용자"(HKLM)로 설치한 옛 버전은
// 관리자 권한 없이 제거할 수 없어 건드리지 않으므로, 두 앱이 함께 뜨면(키보드 훅 2개 → Space 가 두 번 처리) 사용자가 직접 지워야 한다.
// 이 확인은 레지스트리를 읽기만 한다. 키 이름은 electron-builder 가 appId 로 만든 GUID 이고 docs/tauri-migration.md 6절과 같다.

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    KEY_WOW64_64KEY,
};

/// 옛 Electron 설치본의 제거 키(`Uninstall\<GUID>`)
pub const OLD_UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\a32366e0-3e8a-5400-a683-97d78e557198";

/// 키가 있는지만 본다. 64비트 뷰로 연다(electron-builder 는 64비트 뷰에 쓴다. 이 앱은 x64 라 같지만 뜻을 분명히 한다).
pub fn key_exists(root: HKEY, path: &str) -> bool {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey = HKEY::default();
    // SAFETY: wide 는 NUL 로 끝나는 UTF-16, hkey 는 유효한 출력 위치다
    let err = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(wide.as_ptr()),
            None,
            KEY_READ | KEY_WOW64_64KEY,
            &mut hkey,
        )
    };
    if err != ERROR_SUCCESS {
        return false;
    }
    // SAFETY: 방금 연 키다
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    true
}

/// 남아 있는 옛 설치본에 대한 안내 문구. 없으면 None.
pub fn notice(all_users: bool, current_user: bool) -> Option<&'static str> {
    match (all_users, current_user) {
        (true, _) => Some(
            "옛 Electron 버전(모든 사용자 설치)이 남아 있다. 두 앱이 함께 실행되면 Space 가 두 번 처리되므로 \
             관리자 권한으로 Windows 설정 > 앱 에서 옛 Peekaboo 를 제거해야 한다",
        ),
        (false, true) => Some(
            "옛 Electron 버전(현재 사용자 설치)이 제거되지 않고 남아 있다. 두 앱이 함께 실행되면 Space 가 두 번 처리되므로 \
             Windows 설정 > 앱 에서 옛 Peekaboo 를 제거해야 한다",
        ),
        (false, false) => None,
    }
}

/// 모든 사용자 설치본 알림을 이미 보였다는 표식 파일(데이터 폴더). 설정 파일에 필드를 더하지 않으려고 파일로 둔다.
pub const NOTIFIED_MARKER: &str = "legacy-all-users-notified";

const ALL_USERS_TITLE: &str = "이전 버전이 모든 사용자용으로 남아 있습니다";
const ALL_USERS_BODY: &str = "제어판(설정 > 앱)에서 이전 Peekaboo 를 제거해 주세요. 함께 실행되면 Space 가 두 번 처리됩니다.";

/// 모든 사용자 설치본이 있고 아직 알리지 않았으면 알림(`notify`)을 한 번 보내고 표식 파일을 만든다.
/// 표식 폴더를 알 수 없으면(None) 매번 알리지 않도록 알림을 보내지 않는다. 보냈으면 true.
pub fn notify_all_users_once(
    all_users: bool,
    data_dir: Option<&Path>,
    notify: impl FnOnce(&str, &str),
) -> bool {
    let Some(dir) = data_dir else { return false };
    let marker = dir.join(NOTIFIED_MARKER);
    if !all_users || marker.exists() {
        return false;
    }
    // 표식을 먼저 남긴다. 알림이 패닉하거나 표식을 못 만들면 시작할 때마다 알리는 것보다 알리지 않는 쪽이 낫다.
    if let Err(e) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&marker, b"1")) {
        log_warn!("옛 설치본 알림 표식을 만들지 못해 알리지 않는다: {e}");
        return false;
    }
    notify(ALL_USERS_TITLE, ALL_USERS_BODY);
    true
}

/// 실제 레지스트리를 읽어 안내를 로그에 남기고, 모든 사용자 설치본이면 처음 한 번 알림도 보낸다
pub fn check_and_notify(data_dir: Option<&Path>, notify: impl FnOnce(&str, &str)) {
    let all_users = key_exists(HKEY_LOCAL_MACHINE, OLD_UNINSTALL_KEY);
    let current_user = key_exists(HKEY_CURRENT_USER, OLD_UNINSTALL_KEY);
    if let Some(text) = notice(all_users, current_user) {
        log_warn!("{text}");
    }
    notify_all_users_once(all_users, data_dir, notify);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 안내_문구는_남은_설치본에_따라_정해진다() {
        assert!(notice(true, false).unwrap().contains("모든 사용자"));
        assert!(notice(true, true).unwrap().contains("모든 사용자"));
        assert!(notice(false, true).unwrap().contains("현재 사용자"));
        assert_eq!(notice(false, false), None);
    }

    /// 알림은 실제로 띄우지 않고 클로저로 받아 센다. 표식 폴더는 임시 폴더다.
    #[test]
    fn 모든_사용자_설치본_알림은_처음_한_번만() {
        let dir = std::env::temp_dir().join(format!("peekaboo-legacy-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let calls = std::cell::RefCell::new(Vec::<(String, String)>::new());
        let notify = |t: &str, b: &str| calls.borrow_mut().push((t.to_string(), b.to_string()));

        // 모든 사용자 설치본이 없으면 알리지 않고 표식도 만들지 않는다
        assert!(!notify_all_users_once(false, Some(&dir), notify));
        assert!(!dir.join(NOTIFIED_MARKER).exists());
        // 표식 폴더를 모르면 알리지 않는다
        assert!(!notify_all_users_once(true, None, notify));
        assert!(calls.borrow().is_empty());

        // 처음 한 번만 알린다
        assert!(notify_all_users_once(true, Some(&dir), notify));
        assert!(!notify_all_users_once(true, Some(&dir), notify));
        assert_eq!(calls.borrow().len(), 1);
        assert!(calls.borrow()[0].0.contains("모든 사용자용"));
        assert!(calls.borrow()[0].1.contains("제어판"));
        assert!(dir.join(NOTIFIED_MARKER).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 실제 옛 제거 키는 읽지 않는다. 항상 있는 시스템 키와 없는 키로 판정만 확인한다.
    #[test]
    fn 키_존재_판정() {
        assert!(key_exists(
            HKEY_LOCAL_MACHINE,
            r"Software\Microsoft\Windows\CurrentVersion"
        ));
        assert!(!key_exists(
            HKEY_CURRENT_USER,
            r"Software\PeekabooTest\없는-키-legacy"
        ));
    }
}
