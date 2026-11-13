// C# 헬퍼(helper/) 프로세스 연결. Electron 의 src/main/helper-supervisor.ts 와 같은 의미다.
// 프로토콜은 docs/ipc.md(JSON Lines v1), 타입은 src/shared/ipc-types.ts · helper/Ipc/Messages.cs 와 맞춘다.
//
// 이 파일의 앞부분(메시지 타입·줄 파싱·백오프·ping 판정·경로 후보)은 프로세스와 무관한 순수 로직이라 cargo test 로 검증하고,
// 뒷부분(Helper)이 tokio::process 로 헬퍼를 띄우고 감시한다. 헬퍼를 새로 띄우면 `ready` 가 오므로 소비자는 그때
// 설정·상태를 다시 보내면 된다(controller.rs).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Notify};

/// 프로토콜 버전. 모든 메시지에 `v` 로 붙는다.
const PROTOCOL_VERSION: u32 = 1;
const PING_INTERVAL: Duration = Duration::from_secs(2);
/// 연속으로 이만큼 pong 이 없으면 헬퍼를 죽인다(재시작은 감시 루프가 한다)
const MAX_MISSED_PONGS: u32 = 3;
const BACKOFF: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
    Duration::from_secs(30),
];
/// 이만큼 살아 있었으면 백오프 단계를 처음으로 되돌린다
const STABLE_RESET: Duration = Duration::from_secs(60);
/// 정상 종료(`shutdown` → stdin 닫기)를 기다리는 시간. 지나면 강제 종료한다.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);
/// `Helper::stop` 이 감시 루프의 마무리를 기다리는 상한
const STOP_WAIT: Duration = Duration::from_secs(3);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const HELPER_EXE: &str = "peekaboo-helper.exe";

// ---------------------------------------------------------------------------
// 메시지
// ---------------------------------------------------------------------------

/// 미리보기 창 클라이언트 영역 기준 물리 픽셀
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PixelRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PixelRect {
    /// 렌더러가 보낸 값을 믿지 않는다: 모두 유한하고 크기는 음수가 아니어야 한다(Electron 의 isRect 와 같다).
    pub fn is_valid(&self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite())
            && self.width >= 0.0
            && self.height >= 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyName {
    Space,
    Esc,
    Enter,
    Up,
    Down,
    Left,
    Right,
    /// 미리보기에 텍스트가 선택된 채 누른 Ctrl+C
    Copy,
    /// 모르는 키 이름(전방 호환)
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeStatus {
    Shown,
    Unavailable,
    Failed,
}

/// 헬퍼 → 호스트
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HelperMessage {
    Ready {
        version: String,
        pid: u32,
    },
    Key {
        key: KeyName,
        consumed: bool,
    },
    Selection {
        id: Option<i32>,
        paths: Vec<String>,
        focused: Option<String>,
    },
    Pong {
        id: i32,
    },
    Error {
        id: Option<i32>,
        code: String,
        message: String,
    },
    NativePreview {
        id: i32,
        status: NativeStatus,
        message: Option<String>,
    },
    Foreground {
        pid: u32,
        shell: bool,
    },
    /// 모르는 type 은 무시한다(전방 호환)
    #[serde(other)]
    Unknown,
}

/// 호스트 → 헬퍼
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostMessage {
    /// `host_pid` = 이 앱, `host_pids` = 앱 + WebView2 브라우저 프로세스(헬퍼는 둘을 합쳐 본다)
    Config {
        enabled: bool,
        host_pid: u32,
        host_pids: Vec<u32>,
    },
    State {
        preview_open: bool,
    },
    GetSelection {
        id: i32,
    },
    Ping {
        id: i32,
    },
    Shutdown,
    #[serde(rename = "nativePreview.open")]
    NativePreviewOpen {
        id: i32,
        path: String,
        hwnd: i64,
        rect: PixelRect,
        dark: bool,
        /// 컨테이너를 부모의 자식 중 맨 위로 올린다. WebView2 는 컨테이너가 `WRY_WEBVIEW` 아래에 깔려서 true 로 보낸다.
        raise: bool,
    },
    #[serde(rename = "nativePreview.resize")]
    NativePreviewResize {
        id: i32,
        rect: PixelRect,
    },
    #[serde(rename = "nativePreview.close")]
    NativePreviewClose {
        id: i32,
    },
}

impl HostMessage {
    /// `v` 를 붙인 JSON 한 줄(끝에 `\n`)
    pub fn to_line(&self) -> String {
        let mut value = serde_json::to_value(self).expect("HostMessage 는 항상 직렬화된다");
        if let Some(obj) = value.as_object_mut() {
            obj.insert("v".into(), PROTOCOL_VERSION.into());
        }
        let mut line = value.to_string();
        line.push('\n');
        line
    }
}

/// 헬퍼 stdout 한 줄의 해석 결과
#[derive(Debug, PartialEq)]
pub enum Parsed {
    /// 빈 줄
    Empty,
    Message(HelperMessage),
    /// JSON 이 아니거나 필드가 맞지 않는 줄. 로그만 남기고 계속 간다.
    Broken(String),
}

pub fn parse_line(line: &str) -> Parsed {
    if line.trim().is_empty() {
        return Parsed::Empty;
    }
    match serde_json::from_str::<HelperMessage>(line) {
        Ok(msg) => Parsed::Message(msg),
        Err(e) => Parsed::Broken(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// 재시작 백오프·ping 판정
// ---------------------------------------------------------------------------

/// 헬퍼가 `lived` 만큼 살다 죽었을 때 (다시 띄우기까지의 대기, 다음 단계).
/// 단계는 처음 죽을 때 0 이고, `STABLE_RESET` 넘게 살았으면 처음으로 되돌린 뒤 계산한다.
pub fn next_restart(step: usize, lived: Duration) -> (Duration, usize) {
    let step = if lived > STABLE_RESET { 0 } else { step };
    (BACKOFF[step.min(BACKOFF.len() - 1)], step + 1)
}

#[derive(Debug, PartialEq, Eq)]
pub enum PingAction {
    Send(i32),
    /// 응답 없음이 한계를 넘었다. 헬퍼를 죽인다.
    Kill,
}

/// 2초마다 `tick` 을 부른다. 원본과 같이 "무응답이 이미 3번 쌓였으면 죽이고, 아니면 세고 보낸다"이므로
/// 응답이 하나도 없으면 4번째 틱(약 8초)에 죽는다.
#[derive(Debug)]
pub struct PingTracker {
    missed: u32,
    next_id: i32,
}

impl PingTracker {
    pub fn new() -> Self {
        PingTracker {
            missed: 0,
            next_id: 1,
        }
    }

    pub fn tick(&mut self) -> PingAction {
        if self.missed >= MAX_MISSED_PONGS {
            return PingAction::Kill;
        }
        self.missed += 1;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        PingAction::Send(id)
    }

    pub fn pong(&mut self) {
        self.missed = 0;
    }
}

/// ping 주기 타이머. 첫 틱은 한 주기 뒤다.
/// 앱이 오래 멈췄다 풀리면(절전·디버거·프로세스 일시 정지) 기본 동작(Burst)은 밀린 틱을 연달아 몰아서 내 보낸다.
/// 그러면 pong 이 올 틈 없이 ping 3번과 Kill 판정이 한꺼번에 나와 멀쩡한 헬퍼를 죽인다.
/// Delay 는 밀린 틱을 한 번만 내고 다음 틱을 그때부터 한 주기 뒤로 잡아, 재개 뒤에도 pong 을 기다릴 시간이 남는다.
/// (Skip 도 몰아치지는 않지만 원래 격자에 맞추므로 재개 직후 다음 틱이 주기보다 짧게 올 수 있어 Delay 를 쓴다.)
fn ping_interval() -> tokio::time::Interval {
    let mut interval =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

// ---------------------------------------------------------------------------
// 헬퍼 경로
// ---------------------------------------------------------------------------

/// 헬퍼 실행 파일 후보(앞이 우선). 오버라이드가 있으면 그것만 본다.
/// 디버그 빌드는 저장소의 `helper/publish`(`npm run helper:build`)를 먼저 보고, 그다음 실행 파일 옆을 본다.
/// 패키징 경로는 M4 에서 정한다.
pub fn helper_candidates(
    override_path: Option<PathBuf>,
    repo_root: Option<&Path>,
    exe_dir: Option<&Path>,
) -> Vec<PathBuf> {
    if let Some(p) = override_path {
        return vec![p];
    }
    let mut out = Vec::new();
    if let Some(root) = repo_root {
        out.push(root.join("helper").join("publish").join(HELPER_EXE));
    }
    if let Some(dir) = exe_dir {
        out.push(dir.join(HELPER_EXE));
    }
    out
}

/// 현재 환경에서 찾은 헬퍼 경로. 없으면 None.
pub fn resolve_helper_path(override_path: Option<PathBuf>) -> Option<PathBuf> {
    // CARGO_MANIFEST_DIR 은 src-tauri 이므로 저장소 루트는 그 부모다. 릴리스 빌드에는 개발 경로를 넣지 않는다.
    let repo_root = if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent()
    } else {
        None
    };
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    helper_candidates(override_path, repo_root, exe_dir.as_deref())
        .into_iter()
        .find(|p| p.exists())
}

// ---------------------------------------------------------------------------
// 감시
// ---------------------------------------------------------------------------

type OnMessage = Arc<dyn Fn(HelperMessage) + Send + Sync>;

struct Inner {
    /// 지금 살아 있는 헬퍼의 stdin 쓰기 채널. 없으면(안 떴거나 재시작 중) 보내지 않는다.
    tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    stopping: AtomicBool,
    stop: Notify,
    /// 감시 루프가 끝나면 알린다(`stop` 이 기다린다)
    done_tx: std_mpsc::Sender<()>,
    done_rx: Mutex<std_mpsc::Receiver<()>>,
    started: AtomicBool,
}

/// 헬퍼 프로세스 수명 관리: 실행, stdout JSON Lines 파싱, ping 감시, 비정상 종료 시 백오프 재시작.
/// 복제본은 같은 프로세스를 가리킨다.
#[derive(Clone)]
pub struct Helper {
    inner: Arc<Inner>,
}

impl Default for Helper {
    fn default() -> Self {
        let (done_tx, done_rx) = std_mpsc::channel();
        Helper {
            inner: Arc::new(Inner {
                tx: Mutex::new(None),
                stopping: AtomicBool::new(false),
                stop: Notify::new(),
                done_tx,
                done_rx: Mutex::new(done_rx),
                started: AtomicBool::new(false),
            }),
        }
    }
}

impl Helper {
    /// 헬퍼를 띄우고 감시를 시작한다. 두 번 부르면 무시한다. `on_message` 는 감시 태스크에서 불리므로 오래 막지 않는다.
    /// (`pong` 은 여기서 먹고 넘기지 않는다.)
    pub fn start(
        &self,
        path_override: Option<PathBuf>,
        on_message: impl Fn(HelperMessage) + Send + Sync + 'static,
    ) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let on_message: OnMessage = Arc::new(on_message);
        tauri::async_runtime::spawn(async move {
            supervise(&inner, path_override, on_message).await;
            let _ = inner.done_tx.send(());
        });
    }

    /// 헬퍼가 떠 있지 않으면 보내지 않고 false
    pub fn send(&self, msg: &HostMessage) -> bool {
        let guard = self.inner.tx.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(tx) => tx.send(msg.to_line()).is_ok(),
            None => false,
        }
    }

    /// 정상 종료를 요청하고 끝나기를 기다린다(상한 있음). 앱 종료 때 부른다.
    /// 앱이 이 호출 없이 죽어도 stdin 이 닫혀 헬퍼가 스스로 끝난다(helper/Program.cs).
    pub fn stop(&self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        self.inner.stop.notify_one();
        if !self.inner.started.load(Ordering::SeqCst) {
            return;
        }
        let rx = self.inner.done_rx.lock().unwrap_or_else(|e| e.into_inner());
        if rx.recv_timeout(STOP_WAIT).is_err() {
            eprintln!("[peekaboo] 헬퍼 종료를 기다리다 시간이 지났다");
        }
    }
}

/// 어떻게 끝났는지
enum RunEnd {
    /// 헬퍼가 스스로 죽었거나 우리가 죽였다. 다시 띄운다.
    Exited(String),
    /// `stop` 에 따라 정리를 마쳤다.
    Stopped,
}

async fn supervise(inner: &Arc<Inner>, path_override: Option<PathBuf>, on_message: OnMessage) {
    let mut step = 0usize;
    loop {
        if inner.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Some(exe) = resolve_helper_path(path_override.clone()) else {
            // 경로가 없으면 재시작해도 소용없으니 루프를 돌지 않는다
            eprintln!("[peekaboo] 헬퍼 실행 파일을 찾지 못했다(`npm run helper:build` 또는 PEEKABOO_HELPER=<경로>)");
            return;
        };
        eprintln!("[peekaboo] 헬퍼 실행: {}", exe.display());
        let started = Instant::now();
        let end = run_once(inner, &exe, &on_message).await;
        *inner.tx.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let why = match end {
            RunEnd::Stopped => return,
            RunEnd::Exited(why) => why,
        };
        let (delay, next) = next_restart(step, started.elapsed());
        step = next;
        eprintln!(
            "[peekaboo] 헬퍼 종료({why}), {}ms 뒤 다시 띄운다",
            delay.as_millis()
        );
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = inner.stop.notified() => return,
        }
    }
}

async fn write_loop(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(line) = rx.recv().await {
        // 헬퍼가 먼저 죽으면 쓰기가 실패한다 — 종료 처리는 감시 루프가 한다
        if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
            break;
        }
    }
    // stdin 이 여기서 닫힌다(헬퍼는 stdin EOF 로 종료한다)
}

/// 헬퍼를 한 번 띄워 끝날 때까지 감시한다.
async fn run_once(inner: &Arc<Inner>, exe: &Path, on_message: &OnMessage) -> RunEnd {
    let mut child = match Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return RunEnd::Exited(format!("실행 실패: {e}")),
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill().await;
        return RunEnd::Exited("파이프를 얻지 못했다".into());
    };

    let (tx, rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(write_loop(stdin, rx));
    *inner.tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);

    // 줄 단위로 읽되 UTF-8 이 깨진 줄에서 멈추지 않도록 바이트로 읽어 변환한다
    let mut out = BufReader::new(stdout).split(b'\n');
    let mut err = BufReader::new(stderr).split(b'\n');
    let mut out_open = true;
    let mut err_open = true;
    let mut ping = PingTracker::new();
    let mut ticker: Option<tokio::time::Interval> = None;

    loop {
        tokio::select! {
            line = out.next_segment(), if out_open => match line {
                Ok(Some(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    match parse_line(text.trim_end_matches('\r')) {
                        Parsed::Empty => {}
                        Parsed::Broken(e) => {
                            let head: String = text.chars().take(200).collect();
                            eprintln!("[peekaboo] 헬퍼 출력을 해석하지 못했다({e}): {head}");
                        }
                        Parsed::Message(HelperMessage::Pong { .. }) => ping.pong(),
                        Parsed::Message(msg) => {
                            if let HelperMessage::Ready { version, pid } = &msg {
                                eprintln!("[peekaboo] 헬퍼 ready v{version} pid={pid}");
                                ping = PingTracker::new();
                                ticker = Some(ping_interval());
                            }
                            on_message(msg);
                        }
                    }
                }
                _ => out_open = false,
            },
            // stderr 는 끝까지 읽어야 한다(파이프가 차면 헬퍼가 막힌다)
            line = err.next_segment(), if err_open => match line {
                Ok(Some(bytes)) => eprintln!("[helper.exe] {}", String::from_utf8_lossy(&bytes).trim_end()),
                _ => err_open = false,
            },
            _ = async { ticker.as_mut().expect("guard").tick().await }, if ticker.is_some() => {
                match ping.tick() {
                    PingAction::Send(id) => {
                        let line = HostMessage::Ping { id }.to_line();
                        let guard = inner.tx.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(line);
                        }
                    }
                    PingAction::Kill => {
                        eprintln!("[peekaboo] 헬퍼가 응답하지 않아 종료한다");
                        // 종료되면 아래 wait 가지가 재시작으로 이어 준다
                        let _ = child.start_kill();
                    }
                }
            }
            status = child.wait() => {
                return RunEnd::Exited(match status {
                    Ok(s) => format!("{s}"),
                    Err(e) => format!("대기 실패: {e}"),
                });
            }
            _ = inner.stop.notified() => {
                shutdown(inner, &mut child).await;
                return RunEnd::Stopped;
            }
        }
    }
}

/// `shutdown` → stdin 닫기 → 잠깐 기다리고 안 끝나면 강제 종료
async fn shutdown(inner: &Arc<Inner>, child: &mut tokio::process::Child) {
    let tx = inner.tx.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(tx) = tx {
        let _ = tx.send(HostMessage::Shutdown.to_line());
        // tx 를 놓으면 write_loop 가 남은 줄을 쓰고 끝나며 stdin 이 닫힌다
    }
    if tokio::time::timeout(SHUTDOWN_GRACE, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn 줄을_해석한다() {
        assert_eq!(
            parse_line(r#"{"v":1,"type":"ready","version":"0.1.0","pid":1234}"#),
            Parsed::Message(HelperMessage::Ready {
                version: "0.1.0".into(),
                pid: 1234
            })
        );
        assert_eq!(
            parse_line(r#"{"v":1,"type":"key","key":"space","consumed":true}"#),
            Parsed::Message(HelperMessage::Key {
                key: KeyName::Space,
                consumed: true
            })
        );
        assert_eq!(
            parse_line(
                r#"{"v":1,"type":"selection","id":7,"paths":["C:\\a.png"],"focused":"C:\\a.png","source":"explorer","hwnd":329874}"#
            ),
            Parsed::Message(HelperMessage::Selection {
                id: Some(7),
                paths: vec!["C:\\a.png".into()],
                focused: Some("C:\\a.png".into())
            })
        );
        // id·focused 는 없거나 null 이어도 된다
        assert_eq!(
            parse_line(
                r#"{"v":1,"type":"selection","paths":[],"focused":null,"source":"desktop","hwnd":1}"#
            ),
            Parsed::Message(HelperMessage::Selection {
                id: None,
                paths: vec![],
                focused: None
            })
        );
        assert_eq!(
            parse_line(r#"{"v":1,"type":"nativePreview","id":3,"status":"unavailable"}"#),
            Parsed::Message(HelperMessage::NativePreview {
                id: 3,
                status: NativeStatus::Unavailable,
                message: None
            })
        );
        assert_eq!(
            parse_line(r#"{"v":1,"type":"foreground","pid":42,"shell":false}"#),
            Parsed::Message(HelperMessage::Foreground {
                pid: 42,
                shell: false
            })
        );
        assert_eq!(
            parse_line(r#"{"v":1,"type":"error","id":8,"code":"ELEVATED","message":"x"}"#),
            Parsed::Message(HelperMessage::Error {
                id: Some(8),
                code: "ELEVATED".into(),
                message: "x".into()
            })
        );
    }

    #[test]
    fn 알_수_없는_type_과_key_는_무시_대상으로_해석한다() {
        assert_eq!(
            parse_line(r#"{"v":1,"type":"future","x":1}"#),
            Parsed::Message(HelperMessage::Unknown)
        );
        assert_eq!(
            parse_line(r#"{"v":1,"type":"key","key":"tab","consumed":false}"#),
            Parsed::Message(HelperMessage::Key {
                key: KeyName::Unknown,
                consumed: false
            })
        );
    }

    #[test]
    fn 깨진_줄과_빈_줄() {
        assert_eq!(parse_line(""), Parsed::Empty);
        assert_eq!(parse_line("  \r"), Parsed::Empty);
        for bad in [
            "not json",
            "{",
            r#"{"v":1}"#,
            r#"{"v":1,"type":"ready"}"#,
            r#"{"v":1,"type":"key","key":"space"}"#,
            "[1,2]",
        ] {
            assert!(matches!(parse_line(bad), Parsed::Broken(_)), "{bad}");
        }
    }

    #[test]
    fn 보내는_줄은_v_와_camel_case_를_쓴다() {
        let line = HostMessage::Config {
            enabled: false,
            host_pid: 10,
            host_pids: vec![10, 20],
        }
        .to_line();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"v":1,"type":"config","enabled":false,"hostPid":10,"hostPids":[10,20]})
        );

        let v: serde_json::Value =
            serde_json::from_str(&HostMessage::State { preview_open: true }.to_line()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"v":1,"type":"state","previewOpen":true})
        );

        let v: serde_json::Value = serde_json::from_str(&HostMessage::Shutdown.to_line()).unwrap();
        assert_eq!(v, serde_json::json!({"v":1,"type":"shutdown"}));

        let v: serde_json::Value = serde_json::from_str(
            &HostMessage::NativePreviewOpen {
                id: 5,
                path: "C:\\한글 #1.docx".into(),
                hwnd: 66,
                rect: PixelRect {
                    x: 0.0,
                    y: 48.0,
                    width: 800.0,
                    height: 600.0,
                },
                dark: true,
                raise: true,
            }
            .to_line(),
        )
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"v":1,"type":"nativePreview.open","id":5,"path":"C:\\한글 #1.docx",
                "hwnd":66,"rect":{"x":0.0,"y":48.0,"width":800.0,"height":600.0},"dark":true,"raise":true})
        );
        let v: serde_json::Value =
            serde_json::from_str(&HostMessage::NativePreviewClose { id: 5 }.to_line()).unwrap();
        assert_eq!(v["type"], "nativePreview.close");
    }

    /// 지금 바로 나올 수 있는 틱을 전부 소비해 ping 판정에 먹인다(시간은 멈춰 있으므로 시계는 흐르지 않는다)
    async fn drain_ticks(
        ticker: &mut tokio::time::Interval,
        ping: &mut PingTracker,
    ) -> Vec<PingAction> {
        let mut out = Vec::new();
        while tokio::time::timeout(Duration::ZERO, ticker.tick())
            .await
            .is_ok()
        {
            out.push(ping.tick());
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn 오래_멈췄다_풀려도_밀린_ping_을_몰아치지_않아_헬퍼를_죽이지_않는다() {
        let mut ticker = ping_interval();
        let mut ping = PingTracker::new();
        // 정상 동작: 주기마다 ping 하나, pong 이 와 무응답이 쌓이지 않는다
        for _ in 0..3 {
            tokio::time::advance(PING_INTERVAL).await;
            assert!(matches!(
                drain_ticks(&mut ticker, &mut ping).await[..],
                [PingAction::Send(_)]
            ));
            ping.pong();
        }
        // 앱이 20초 멈췄다 풀린다(밀린 틱이 10개). Kill 판정은 없고 ping 은 한 번만 나간다.
        tokio::time::advance(secs(20)).await;
        let got = drain_ticks(&mut ticker, &mut ping).await;
        assert!(
            matches!(got[..], [PingAction::Send(_)]),
            "밀린 틱을 몰아치면 안 된다: {got:?}"
        );
        // 다음 틱은 재개 시점부터 한 주기 뒤라 그 사이 pong 을 받을 수 있다
        tokio::time::advance(PING_INTERVAL - Duration::from_millis(1)).await;
        assert!(drain_ticks(&mut ticker, &mut ping).await.is_empty());
        ping.pong();
        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(matches!(
            drain_ticks(&mut ticker, &mut ping).await[..],
            [PingAction::Send(_)]
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn 진짜_무응답이면_멈춤과_무관하게_네_번째_틱에_죽인다() {
        let mut ticker = ping_interval();
        let mut ping = PingTracker::new();
        let mut actions = Vec::new();
        for _ in 0..4 {
            tokio::time::advance(PING_INTERVAL).await;
            actions.extend(drain_ticks(&mut ticker, &mut ping).await);
        }
        assert_eq!(actions.last(), Some(&PingAction::Kill));
        assert_eq!(actions.len(), 4);
    }

    #[test]
    fn 백오프는_1_2_4_8_15_30_초에서_멈춘다() {
        let mut step = 0;
        let mut delays = Vec::new();
        for _ in 0..8 {
            let (d, next) = next_restart(step, secs(1));
            delays.push(d);
            step = next;
        }
        assert_eq!(
            delays,
            [1, 2, 4, 8, 15, 30, 30, 30].map(Duration::from_secs)
        );
    }

    #[test]
    fn 오래_살았으면_백오프를_처음으로_되돌린다() {
        assert_eq!(next_restart(4, secs(61)), (secs(1), 1));
        // 정확히 60초는 아직 아니다(원본은 `> 60_000`)
        assert_eq!(next_restart(4, secs(60)), (secs(15), 5));
        assert_eq!(next_restart(0, secs(0)), (secs(1), 1));
    }

    #[test]
    fn 응답이_없으면_네_번째_틱에_죽인다() {
        let mut p = PingTracker::new();
        assert_eq!(p.tick(), PingAction::Send(1));
        assert_eq!(p.tick(), PingAction::Send(2));
        assert_eq!(p.tick(), PingAction::Send(3));
        assert_eq!(p.tick(), PingAction::Kill);
        assert_eq!(p.tick(), PingAction::Kill);
    }

    #[test]
    fn pong_이_오면_무응답_횟수가_초기화된다() {
        let mut p = PingTracker::new();
        p.tick();
        p.tick();
        p.tick();
        p.pong();
        assert_eq!(p.tick(), PingAction::Send(4));
        assert_eq!(p.tick(), PingAction::Send(5));
        assert_eq!(p.tick(), PingAction::Send(6));
        assert_eq!(p.tick(), PingAction::Kill);
    }

    #[test]
    fn 헬퍼_경로_후보() {
        let root = Path::new(r"C:\repo");
        let exe = Path::new(r"C:\app");
        assert_eq!(
            helper_candidates(Some(PathBuf::from(r"D:\h.exe")), Some(root), Some(exe)),
            vec![PathBuf::from(r"D:\h.exe")],
            "오버라이드는 다른 후보를 보지 않는다"
        );
        assert_eq!(
            helper_candidates(None, Some(root), Some(exe)),
            vec![
                PathBuf::from(r"C:\repo\helper\publish\peekaboo-helper.exe"),
                PathBuf::from(r"C:\app\peekaboo-helper.exe"),
            ]
        );
        assert_eq!(
            helper_candidates(None, None, Some(exe)),
            vec![PathBuf::from(r"C:\app\peekaboo-helper.exe")]
        );
        assert!(helper_candidates(None, None, None).is_empty());
    }

    #[test]
    fn 픽셀_사각형_검증() {
        let ok = PixelRect {
            x: -5.0,
            y: 0.0,
            width: 0.0,
            height: 10.0,
        };
        assert!(ok.is_valid());
        for bad in [
            PixelRect { width: -1.0, ..ok },
            PixelRect { height: -0.5, ..ok },
            PixelRect { x: f64::NAN, ..ok },
            PixelRect {
                width: f64::INFINITY,
                ..ok
            },
        ] {
            assert!(!bad.is_valid(), "{bad:?}");
        }
    }

    #[test]
    fn 안_떠_있으면_보내지_않고_false() {
        let helper = Helper::default();
        assert!(!helper.send(&HostMessage::Shutdown));
        // 시작하지 않은 헬퍼의 stop 은 기다리지 않는다
        helper.stop();
    }
}
