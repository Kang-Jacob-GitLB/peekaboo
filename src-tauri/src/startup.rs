// Windows 시작 시 실행. HKCU `Software\Microsoft\Windows\CurrentVersion\Run` 의 값을 직접 관리한다.
// 값 이름과 값 모양은 Electron(`app.setLoginItemSettings`)이 쓰던 것과 같아(`"<exe 경로>"`) 옛 설치본이 등록한 값을 이어받는다.
// 켜짐 여부는 OS 상태(레지스트리)가 단일 출처다 — 설정 파일에 따로 저장하지 않는다(Electron settings.ts 의 getLaunchAtStartup).
//
// 키 경로·값 이름을 인자로 받는 이유: 시험이 실제 Run 키를 건드리지 않고 임시 키(`HKCU\Software\PeekabooTest\…`)로만 하게 하려는 것이다.
//
// "켜짐" 판정: 값이 있고, 그 명령줄의 실행 파일이 현재 exe 와 같고(대소문자·슬래시 방향 무시), 인자가 없을 때.
// 다른 경로(예: 옛 Electron 설치본)를 가리키는 값은 꺼짐으로 본다 — 이 앱을 켜 두는 것이 아니기 때문이다.
// 작업 관리자 "시작 앱"의 사용 안 함 상태(StartupApproved)는 켜짐 판정에는 보지 않는다.
// 다만 켤 때는 같은 이름의 StartupApproved 값이 있으면 "사용" 으로 되돌린다 — 사용자가 작업 관리자에서 꺼 둔 옛 등록이
// 남아 있으면 Run 값만 써서는 실제로 시작되지 않기 때문이다(Electron `setLoginItemSettings({ openAtLogin: true })` 와 같은 효과).
// StartupApproved\Run 은 문서화되지 않은 셸 영역이다(Microsoft Learn 에 없음). 값은 REG_BINARY 12바이트이고
// 앞 4바이트의 최하위 비트가 0 이면 사용, 1 이면 사용 안 함이며 나머지 8바이트는 끈 시각(FILETIME)이다.
// 값을 지워도 "사용" 이 되지만 확인된 근거가 없어, 작업 관리자가 켤 때 쓰는 모양(`02 00 00 00` + 0 8바이트)을 직접 쓴다.
// 값이 없으면 사용자가 손댄 적이 없다는 뜻이라 아무것도 하지 않는다. 끌 때는 Run 값만 지운다.

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_BINARY, REG_OPTION_NON_VOLATILE, REG_SZ,
    REG_VALUE_TYPE,
};

/// 실제 Run 키
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 작업 관리자 "시작 앱" 의 사용·사용 안 함 상태가 사는 키(값 이름은 Run 과 같다)
pub const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// StartupApproved 값의 "사용" 모양: `02 00 00 00` + 끈 시각 0(8바이트)
const APPROVED_ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
/// Electron 이 쓰던 값 이름(앱 identifier). 바꾸면 옛 등록을 못 이어받는다.
pub const VALUE_NAME: &str = "io.github.kang-jacob-gitlb.peekaboo";

/// Run 값에 쓰는 명령줄: 따옴표로 감싼 exe 경로
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// 명령줄에서 실행 파일 경로와 남은 인자를 나눈다. 따옴표로 감쌌으면 닫는 따옴표까지, 아니면 첫 공백까지.
fn split_command(cmd: &str) -> (&str, &str) {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        return match rest.split_once('"') {
            Some((exe, args)) => (exe, args.trim()),
            None => (rest, ""),
        };
    }
    match cmd.split_once(char::is_whitespace) {
        Some((exe, args)) => (exe, args.trim()),
        None => (cmd, ""),
    }
}

fn normalize(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// 저장된 값이 이 exe 를 인자 없이 실행하는 명령줄인가
pub fn command_matches(stored: &str, exe: &Path) -> bool {
    let (stored_exe, args) = split_command(stored);
    args.is_empty() && normalize(stored_exe) == normalize(&exe.display().to_string())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 열려 있는 레지스트리 키를 닫는 가드
struct OpenKey(HKEY);

impl Drop for OpenKey {
    fn drop(&mut self) {
        // SAFETY: 이 가드가 소유한, 열려 있는 키다
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn open(
    key: &str,
    access: windows::Win32::System::Registry::REG_SAM_FLAGS,
) -> Result<Option<OpenKey>, String> {
    let key = wide(key);
    let mut hkey = HKEY::default();
    // SAFETY: key 는 NUL 로 끝나는 UTF-16, hkey 는 유효한 출력 위치다
    let err = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            access,
            &mut hkey,
        )
    };
    match err {
        ERROR_SUCCESS => Ok(Some(OpenKey(hkey))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        e => Err(format!("RegOpenKeyExW 실패: {}", e.0)),
    }
}

/// 값의 종류와 원시 바이트를 읽는다. 키나 값이 없으면 None.
fn read_raw(key: &str, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let opened = open(key, KEY_READ).ok()??;
    let name = wide(name);
    let mut kind = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    // SAFETY: 크기만 묻는다(data 가 None)
    let err = unsafe {
        RegQueryValueExW(
            opened.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    // SAFETY: buf 는 size 바이트
    let err = unsafe {
        RegQueryValueExW(
            opened.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if err != ERROR_SUCCESS {
        return None;
    }
    buf.truncate(size as usize);
    Some((kind, buf))
}

/// 값을 문자열로 읽는다. 키나 값이 없거나 문자열(REG_SZ)이 아니면 None.
pub fn read_value(key: &str, name: &str) -> Option<String> {
    let (kind, buf) = read_raw(key, name)?;
    if kind != REG_SZ {
        return None;
    }
    let units: Vec<u16> = buf
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

fn write_value(key: &str, name: &str, value: &str) -> Result<(), String> {
    let bytes: Vec<u8> = wide(value).iter().flat_map(|u| u.to_le_bytes()).collect();
    write_raw(key, name, REG_SZ, &bytes)
}

fn write_raw(key: &str, name: &str, kind: REG_VALUE_TYPE, bytes: &[u8]) -> Result<(), String> {
    let key = wide(key);
    let mut hkey = HKEY::default();
    // SAFETY: key 는 NUL 로 끝나는 UTF-16, hkey 는 유효한 출력 위치다
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    };
    if err != ERROR_SUCCESS {
        return Err(format!("RegCreateKeyExW 실패: {}", err.0));
    }
    let opened = OpenKey(hkey);
    let name = wide(name);
    // SAFETY: bytes 는 kind 에 맞는 값 데이터다
    let err = unsafe { RegSetValueExW(opened.0, PCWSTR(name.as_ptr()), None, kind, Some(bytes)) };
    if err != ERROR_SUCCESS {
        return Err(format!("RegSetValueExW 실패: {}", err.0));
    }
    Ok(())
}

fn delete_value(key: &str, name: &str) -> Result<(), String> {
    let Some(opened) = open(key, KEY_WRITE)? else {
        return Ok(()); // 키가 없으면 지울 것도 없다
    };
    let name = wide(name);
    // SAFETY: name 은 NUL 로 끝나는 UTF-16
    match unsafe { RegDeleteValueW(opened.0, PCWSTR(name.as_ptr())) } {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        e => Err(format!("RegDeleteValueW 실패: {}", e.0)),
    }
}

/// 시작 시 실행이 켜져 있는가(OS 상태)
pub fn is_enabled(key: &str, name: &str, exe: &Path) -> bool {
    read_value(key, name).is_some_and(|stored| command_matches(&stored, exe))
}

/// 시작 시 실행을 켜거나 끈다. 켤 때 `approved_key` 에 같은 이름의 값이 있으면 "사용" 으로 되돌린다.
pub fn set_enabled(
    key: &str,
    approved_key: &str,
    name: &str,
    exe: &Path,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        write_value(key, name, &run_command(exe))?;
        if read_raw(approved_key, name).is_some() {
            write_raw(approved_key, name, REG_BINARY, &APPROVED_ENABLED)?;
        }
        Ok(())
    } else {
        delete_value(key, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::{RegDeleteKeyW, RegDeleteTreeW};

    /// 시험 전용 키(`HKCU\Software\PeekabooTest\<이름>-<pid>`). 쓰고 나면(실패해도) 지운다.
    /// 실제 Run 키는 건드리지 않는다.
    struct TestKey(String);

    impl TestKey {
        fn new(name: &str) -> Self {
            let key = TestKey(format!(r"{TEST_ROOT}\{name}-{}\Run", std::process::id()));
            key.cleanup();
            key
        }

        fn path(&self) -> &str {
            &self.0
        }

        fn cleanup(&self) {
            // 고유 하위 키(…\<이름>-<pid>) 전체를 지운다
            let root = self.0.trim_end_matches(r"\Run");
            let root = wide(root);
            let parent = wide(TEST_ROOT);
            // SAFETY: root·parent 는 NUL 로 끝나는 UTF-16
            unsafe {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(root.as_ptr()));
                // 비어 있는 시험 루트(Software\PeekabooTest)도 치운다. 다른 시험이 쓰는 중이면(비어 있지 않으면) 실패하므로 무시한다.
                let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(parent.as_ptr()));
            }
        }
    }

    const TEST_ROOT: &str = r"Software\PeekabooTest";

    impl Drop for TestKey {
        fn drop(&mut self) {
            self.cleanup();
        }
    }

    /// 같은 시험 하위 키 안의 StartupApproved 대용 키(실제 StartupApproved 는 건드리지 않는다)
    fn approved(key: &TestKey) -> String {
        key.path().replace(r"\Run", r"\Approved")
    }

    const EXE: &str = r"C:\Program Files\Peekaboo\Peekaboo.exe";

    #[test]
    fn 값_모양은_따옴표로_감싼_경로() {
        assert_eq!(run_command(Path::new(EXE)), format!("\"{EXE}\""));
    }

    #[test]
    fn 명령줄_일치_판정() {
        let exe = Path::new(EXE);
        assert!(command_matches(&format!("\"{EXE}\""), exe));
        // 대소문자·슬래시 방향은 무시
        assert!(command_matches(
            r#""c:/program files/peekaboo/PEEKABOO.EXE""#,
            exe
        ));
        // 따옴표 없는 경로는 공백에서 끊기므로 공백이 든 경로와는 일치하지 않는다
        assert!(!command_matches(EXE, exe));
        assert!(command_matches(r"C:\a\b.exe", Path::new(r"C:\a\b.exe")));
        // 인자가 있거나 다른 exe 면 꺼짐
        assert!(!command_matches(&format!("\"{EXE}\" --x"), exe));
        assert!(!command_matches(
            r#""C:\Users\u\AppData\Local\Programs\Peekaboo\Peekaboo.exe""#,
            exe
        ));
        assert!(!command_matches("", exe));
    }

    #[test]
    fn 켜고_끄면_값이_생기고_사라진다() {
        let key = TestKey::new("toggle");
        let exe = Path::new(EXE);
        assert!(!is_enabled(key.path(), VALUE_NAME, exe));

        set_enabled(key.path(), &approved(&key), VALUE_NAME, exe, true).unwrap();
        assert_eq!(
            read_value(key.path(), VALUE_NAME).as_deref(),
            Some(format!("\"{EXE}\"").as_str())
        );
        assert!(is_enabled(key.path(), VALUE_NAME, exe));
        // 다시 켜도 같은 값
        set_enabled(key.path(), &approved(&key), VALUE_NAME, exe, true).unwrap();
        assert!(is_enabled(key.path(), VALUE_NAME, exe));

        set_enabled(key.path(), &approved(&key), VALUE_NAME, exe, false).unwrap();
        assert!(!is_enabled(key.path(), VALUE_NAME, exe));
        assert_eq!(read_value(key.path(), VALUE_NAME), None);
        // 이미 없는 값을 지워도 오류가 아니다
        set_enabled(key.path(), &approved(&key), VALUE_NAME, exe, false).unwrap();
    }

    #[test]
    fn 키가_없을_때_끄기는_성공한다() {
        let key = TestKey::new("nokey");
        set_enabled(
            key.path(),
            &approved(&key),
            VALUE_NAME,
            Path::new(EXE),
            false,
        )
        .unwrap();
        assert!(!is_enabled(key.path(), VALUE_NAME, Path::new(EXE)));
    }

    #[test]
    fn 다른_경로를_가리키는_값은_꺼짐으로_본다() {
        let key = TestKey::new("other");
        set_enabled(
            key.path(),
            &approved(&key),
            VALUE_NAME,
            Path::new(r"D:\old\Peekaboo.exe"),
            true,
        )
        .unwrap();
        assert!(!is_enabled(key.path(), VALUE_NAME, Path::new(EXE)));
        // 같은 이름으로 다시 켜면 현재 경로로 바뀐다
        set_enabled(
            key.path(),
            &approved(&key),
            VALUE_NAME,
            Path::new(EXE),
            true,
        )
        .unwrap();
        assert!(is_enabled(key.path(), VALUE_NAME, Path::new(EXE)));
    }

    #[test]
    fn 한글_경로도_왕복한다() {
        let key = TestKey::new("hangul");
        let exe = Path::new(r"C:\사용자\피카부\Peekaboo.exe");
        set_enabled(key.path(), &approved(&key), VALUE_NAME, exe, true).unwrap();
        assert!(is_enabled(key.path(), VALUE_NAME, exe));
    }

    #[test]
    fn 켜면_사용_안_함_상태가_사용으로_돌아온다() {
        let key = TestKey::new("approved");
        let approved = approved(&key);
        let exe = Path::new(EXE);
        let disabled: [u8; 12] = [3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8];
        write_raw(&approved, VALUE_NAME, REG_BINARY, &disabled).unwrap();

        // 끌 때는 StartupApproved 를 건드리지 않는다
        set_enabled(key.path(), &approved, VALUE_NAME, exe, false).unwrap();
        assert_eq!(
            read_raw(&approved, VALUE_NAME),
            Some((REG_BINARY, disabled.to_vec()))
        );

        set_enabled(key.path(), &approved, VALUE_NAME, exe, true).unwrap();
        assert!(is_enabled(key.path(), VALUE_NAME, exe));
        assert_eq!(
            read_raw(&approved, VALUE_NAME),
            Some((REG_BINARY, APPROVED_ENABLED.to_vec()))
        );
    }

    #[test]
    fn 사용_안_함_값이_없으면_만들지_않는다() {
        let key = TestKey::new("noapproved");
        let approved = approved(&key);
        set_enabled(key.path(), &approved, VALUE_NAME, Path::new(EXE), true).unwrap();
        assert_eq!(read_raw(&approved, VALUE_NAME), None);
    }
}
