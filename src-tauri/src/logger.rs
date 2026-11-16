// 로그 파일. Electron 의 src/main/logger.ts 와 같은 형식·정책이다.
//   한 줄: `2026-11-16T01:02:03.456Z [LEVEL] [scope] 메시지`
//   파일: `<데이터 폴더>\logs\peekaboo.log`, 2MB 를 넘으면 `peekaboo.log.1` 로 한 번 회전(이전 .1 은 덮어쓴다)
//   헬퍼 stderr 처럼 이미 레벨·시간이 붙은 줄은 레벨 `RAW` 로 그대로 남긴다.
// 디버그 빌드는 stderr 에도 같은 줄을 쓴다(Electron 의 `!app.isPackaged` 와 같다). 로그 실패로 앱을 멈추지 않는다.
//
// `init` 전(단위 테스트 등)에는 파일에 쓰지 않고 stderr 에만 쓴다.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 이 크기를 넘으면 회전한다
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
const FILE_NAME: &str = "peekaboo.log";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
    Raw,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
            Level::Raw => "RAW",
        }
    }
}

struct Logger {
    file: PathBuf,
    /// 쓰기·회전을 직렬화한다
    lock: Mutex<()>,
    also_stderr: bool,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// 로그 폴더를 정한다. 한 번만 유효하다. `also_stderr` 는 디버그 빌드에서 켠다.
pub fn init(dir: &Path, also_stderr: bool) {
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("로그 폴더를 만들지 못했다({}): {e}", dir.display());
    }
    let _ = LOGGER.set(Logger {
        file: dir.join(FILE_NAME),
        lock: Mutex::new(()),
        also_stderr,
    });
}

/// 매크로가 부른다. `module_path!()` 의 마지막 마디를 scope 로 쓴다.
pub fn write(level: Level, module: &str, msg: &str) {
    write_scoped(level, scope_of(module), msg);
}

pub fn write_scoped(level: Level, scope: &str, msg: &str) {
    let line = format_line(now_millis(), level, scope, msg);
    match LOGGER.get() {
        Some(logger) => logger.append(&line),
        None => write_stderr(&line),
    }
}

/// 로그에 남길 URL 의 최대 글자 수
const URL_MAX_CHARS: usize = 120;

/// 차단한 URL 을 로그에 남기기 전에 앞 120자만 남기고, 잘렸으면 `…` 를 붙인다.
/// 미리보기 대상 파일이 만든 URL 은 길이·내용을 신뢰할 수 없어(쿼리에 토큰이 실릴 수도 있다) 통째로 적지 않는다.
pub fn clip_url(url: &str) -> String {
    match url.char_indices().nth(URL_MAX_CHARS) {
        Some((end, _)) => format!("{}…", &url[..end]),
        None => url.to_string(),
    }
}

/// stderr 가 닫혀 있어도(GUI 앱은 콘솔이 없다) 패닉하지 않도록 실패를 무시하고 쓴다
fn write_stderr(line: &str) {
    let _ = std::io::stderr().write_all(line.as_bytes());
}

/// 이미 레벨·시간이 붙은 줄(헬퍼 stderr)을 그대로 남긴다
pub fn raw(scope: &str, msg: &str) {
    write_scoped(Level::Raw, scope, msg);
}

impl Logger {
    fn append(&self, line: &str) {
        if self.also_stderr {
            write_stderr(line);
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        append_with_rotation(&self.file, line, MAX_BYTES);
    }
}

/// `peekaboo::preview` → `preview`, `peekaboo` → `main`
fn scope_of(module: &str) -> &str {
    match module.rsplit_once("::") {
        Some((_, last)) => last,
        None => "main",
    }
}

/// 로그 한 줄(끝에 개행 포함)
pub fn format_line(epoch_ms: u64, level: Level, scope: &str, msg: &str) -> String {
    format!(
        "{} [{}] [{scope}] {msg}\n",
        iso_utc(epoch_ms),
        level.as_str()
    )
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// 에포크 밀리초를 `YYYY-MM-DDTHH:MM:SS.mmmZ`(JS 의 `toISOString`)로 바꾼다.
pub fn iso_utc(epoch_ms: u64) -> String {
    let ms = epoch_ms % 1000;
    let secs = epoch_ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // 1970-01-01 기준 일수 → 그레고리력 날짜(Howard Hinnant 의 civil_from_days)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// 파일이 `max` 바이트를 넘었으면 `.1` 로 옮긴 뒤 한 줄을 덧붙인다. 실패는 삼킨다.
fn append_with_rotation(path: &Path, line: &str, max: u64) {
    if fs::metadata(path).is_ok_and(|m| m.len() > max) {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        let rotated = PathBuf::from(rotated);
        // Rust 의 fs::rename 은 Windows 에서도 대상이 있으면 덮어쓴다(MoveFileExW + REPLACE_EXISTING). Node 의 renameSync 와 같다.
        let _ = fs::rename(path, &rotated);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logger::write($crate::logger::Level::Info, module_path!(), &format!($($arg)*))
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logger::write($crate::logger::Level::Warn, module_path!(), &format!($($arg)*))
    };
}

macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logger::write($crate::logger::Level::Error, module_path!(), &format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("peekaboo-logger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn url_은_120자까지만_남기고_잘리면_말줄임표() {
        let short = "a".repeat(120);
        assert_eq!(clip_url(&short), short);
        let long = "한".repeat(121);
        assert_eq!(clip_url(&long), format!("{}…", "한".repeat(120)));
        assert_eq!(clip_url(""), "");
    }

    #[test]
    fn iso_시각은_toisostring_과_같다() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        // 2026-11-16T01:02:03.456Z
        assert_eq!(iso_utc(1_794_790_923_456), "2026-11-16T01:02:03.456Z");
        // 윤일
        assert_eq!(iso_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_utc(1_709_164_799_999), "2024-02-28T23:59:59.999Z");
        assert_eq!(iso_utc(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
        // 연말
        assert_eq!(iso_utc(1_767_225_599_000), "2025-12-31T23:59:59.000Z");
    }

    #[test]
    fn 한_줄_형식() {
        assert_eq!(
            format_line(0, Level::Warn, "helper", "헬퍼 종료"),
            "1970-01-01T00:00:00.000Z [WARN] [helper] 헬퍼 종료\n"
        );
        assert_eq!(
            format_line(0, Level::Raw, "helper", "x"),
            "1970-01-01T00:00:00.000Z [RAW] [helper] x\n"
        );
    }

    #[test]
    fn scope_는_모듈_마지막_마디() {
        assert_eq!(scope_of("peekaboo::preview"), "preview");
        assert_eq!(scope_of("peekaboo::a::b"), "b");
        assert_eq!(scope_of("peekaboo"), "main");
    }

    #[test]
    fn 덧붙이고_크기를_넘으면_한_번_회전한다() {
        let dir = temp_dir("rotate");
        let path = dir.join(FILE_NAME);
        append_with_rotation(&path, "a\n", 10);
        append_with_rotation(&path, "b\n", 10);
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\nb\n");

        // 10바이트를 넘긴 다음 쓰기에서 회전한다
        append_with_rotation(&path, "cccccccc\n", 10);
        assert!(!dir.join("peekaboo.log.1").exists());
        append_with_rotation(&path, "d\n", 10);
        assert_eq!(fs::read_to_string(&path).unwrap(), "d\n");
        assert_eq!(
            fs::read_to_string(dir.join("peekaboo.log.1")).unwrap(),
            "a\nb\ncccccccc\n"
        );

        // 두 번째 회전은 .1 을 덮어쓴다
        append_with_rotation(&path, "eeeeeeeeeee\n", 10);
        append_with_rotation(&path, "f\n", 10);
        assert_eq!(fs::read_to_string(&path).unwrap(), "f\n");
        assert_eq!(
            fs::read_to_string(dir.join("peekaboo.log.1")).unwrap(),
            "d\neeeeeeeeeee\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 폴더가_없으면_쓰기를_조용히_포기한다() {
        let path = std::env::temp_dir()
            .join(format!("peekaboo-logger-none-{}", std::process::id()))
            .join("x")
            .join(FILE_NAME);
        append_with_rotation(&path, "a\n", 10);
        assert!(!path.exists());
    }
}
