// 설정·로그가 사는 폴더. Electron 의 `app.getPath('userData')`(= `%APPDATA%\Peekaboo`)와 같은 위치를 쓴다.
// 전환 뒤에도 설정이 이어지게 하려는 것이다(docs/tauri-migration.md 6절).
//
// 디버그 빌드는 `%APPDATA%\Peekaboo-dev` 를 쓴다. 개발·시험 중에 설치된 Peekaboo 의 설정·로그를 건드리지 않기 위해서다.
// Tauri 의 `app_data_dir()` 은 identifier 이름의 폴더를 주므로 쓰지 않는다.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

const RELEASE_FOLDER: &str = "Peekaboo";
const DEBUG_FOLDER: &str = "Peekaboo-dev";

/// 빌드 종류별 폴더 이름
pub fn folder_name(debug: bool) -> &'static str {
    if debug {
        DEBUG_FOLDER
    } else {
        RELEASE_FOLDER
    }
}

/// `%APPDATA%` 아래의 데이터 폴더
pub fn data_dir_in(appdata: &Path, debug: bool) -> PathBuf {
    appdata.join(folder_name(debug))
}

/// 로밍 AppData 폴더(`%APPDATA%` 와 같은 위치).
/// 환경 변수는 부모 프로세스가 바꿔 넘길 수 있어 믿지 않고 셸에 직접 묻는다.
fn roaming_appdata() -> Option<PathBuf> {
    // SAFETY: 성공하면 셸이 CoTaskMem 으로 할당한 NUL 종료 UTF-16 문자열을 돌려주고, 복사한 뒤 CoTaskMemFree 로 해제한다.
    unsafe {
        let pwstr = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None).ok()?;
        let path = pwstr.to_string().ok().filter(|s| !s.is_empty());
        CoTaskMemFree(Some(pwstr.as_ptr().cast_const().cast()));
        path.map(PathBuf::from)
    }
}

/// 데이터 폴더(로밍 AppData 를 알 수 없으면 None)
pub fn data_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| roaming_appdata().map(|d| data_dir_in(&d, cfg!(debug_assertions))))
        .as_deref()
}

pub fn settings_file_in(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

pub fn log_dir_in(dir: &Path) -> PathBuf {
    dir.join("logs")
}

/// 로그 폴더(`…\logs`)
pub fn log_dir() -> Option<PathBuf> {
    data_dir().map(log_dir_in)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 디버그는_dev_폴더를_쓴다() {
        let appdata = Path::new(r"C:\Users\x\AppData\Roaming");
        assert_eq!(
            data_dir_in(appdata, false),
            PathBuf::from(r"C:\Users\x\AppData\Roaming\Peekaboo")
        );
        assert_eq!(
            data_dir_in(appdata, true),
            PathBuf::from(r"C:\Users\x\AppData\Roaming\Peekaboo-dev")
        );
    }

    #[test]
    fn 로밍_appdata_는_절대_경로() {
        let dir = roaming_appdata().expect("셸이 로밍 AppData 를 알려 준다");
        assert!(dir.is_absolute(), "{dir:?}");
        assert!(dir.is_dir(), "{dir:?}");
    }

    #[test]
    fn 설정과_로그_경로() {
        let dir = Path::new(r"D:\d");
        assert_eq!(settings_file_in(dir), PathBuf::from(r"D:\d\settings.json"));
        assert_eq!(log_dir_in(dir), PathBuf::from(r"D:\d\logs"));
    }
}
