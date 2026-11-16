// 자동 업데이트. Electron 의 src/main/updater.ts 와 같은 흐름이다.
//   시작 10초 뒤 한 번, 이후 6시간마다 확인 → 새 버전이면 내려받아 메모리에 보관 → 트레이의 "다시 시작하여 업데이트"
//   (또는 "종료")에서 설치. 알림은 안내만 하고 클릭으로 설치하지 않는다(계획서 1절 결정).
//
// `tauri-plugin-updater` 는 Rust API 로만 쓴다: 플러그인이 등록하는 JS 커맨드는 capability 에 권한을 주지 않아 웹뷰에서 못 부른다.
// 설치는 내려받은 NSIS 설치본을 `/P /UPDATE /R`(passive, 설치 뒤 재실행)로 실행하고 이 프로세스는 곧바로 끝난다
// (플러그인 updater.rs 의 `install_inner` 가 `ShellExecuteW` 뒤에 `std::process::exit(0)`). 그래서 종료 훅(`before_exit`)에서
// 헬퍼를 먼저 내린다 — `RunEvent::Exit` 는 오지 않는다.
//
// 켜지는 조건(`disabled_reason`): 릴리스 빌드, 버전이 0.0.0 계열이 아님, 서명 공개키가 있음. 공개키는 저장소의 src-tauri/updater.pub 이고
// 빌드할 때 `plugins.updater.pubkey` 로 주입한다(scripts/tauri-build.mjs). 파일이 없어 주입되지 않으면 플러그인을 등록하지 않는다(main.rs).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};

/// 시작 직후 첫 확인까지(부팅 직후 네트워크·디스크 경합을 피한다)
pub const FIRST_CHECK: Duration = Duration::from_secs(10);
/// 이후 주기적 확인 — 트레이에 오래 상주하므로
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// 확인·내려받기 요청 하나의 제한 시간(본문 받기 포함). 플러그인 기본값은 제한이 없어, 연결이 멈추면 상태가 영영
/// `Checking`/`Downloading` 에 머물고(진행 중이면 다음 확인을 건너뛴다) 트레이 항목이 계속 비활성이 된다.
/// 확인 요청에는 `UpdaterBuilder::timeout` 이 적용되지만 내려받기는 `Update::timeout` 을 따로 정해야 한다.
/// tauri-plugin-updater 2.14.1 의 `update_from_release`(updater.rs:735)가 `timeout: None` 으로 `Update` 를 만들고
/// `download`(updater.rs:849)는 그 필드만 읽는다. 그래서 `check()` 결과에 직접 넣는다.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// 내려받는 설치본의 크기 상한. `download` 는 본문 전체를 메모리 버퍼에 쌓으므로(updater.rs:869-874) 상한이 없으면
/// 피드를 바꿔치는 쪽이 끝없는 본문으로 메모리를 채울 수 있다. 지금 설치본은 수십 MB 이하다.
pub const MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Checking,
    Latest,
    Downloading { version: String, percent: u8 },
    Ready { version: String },
    Error,
}

/// 업데이트를 켤 수 없는 이유. 켤 수 있으면 None.
pub fn disabled_reason(debug: bool, version: &str, pubkey: &str) -> Option<&'static str> {
    if debug {
        return Some("디버그 빌드");
    }
    // 로컬 빌드(0.0.0)와 CI 점검 빌드(0.0.0-ci.N)는 검증 중인 빌드가 최신 릴리스로 덮어써지지 않게 끈다
    if version == "0.0.0" || version.starts_with("0.0.0-") {
        return Some("개발 버전(0.0.0)");
    }
    if pubkey.trim().is_empty() {
        return Some("서명 공개키가 없음");
    }
    None
}

/// 내려받은 설치본. 설치할 때까지 메모리에 둔다.
struct Ready {
    update: Update,
    bytes: Vec<u8>,
}

struct Inner {
    state: UpdateState,
    /// 사람이 트레이에서 누른 확인이면 결과(최신·오류)를 알림으로 알려 준다. 자동 확인은 조용히 넘어간다.
    manual: bool,
    notified_version: Option<String>,
    ready: Option<Ready>,
}

type NotifyFn = Box<dyn Fn(&str, &str) + Send + Sync>;

/// 생성할 때 넘기는 것들. 알림·트레이 갱신·종료 정리를 클로저로 받아, 시험이 실제 알림이나 설치를 일으키지 않게 한다.
pub struct Hooks {
    /// 알림(제목, 본문)
    pub notify: NotifyFn,
    /// 상태가 바뀌어 트레이 메뉴를 다시 만들어야 할 때
    pub on_change: Box<dyn Fn() + Send + Sync>,
    /// 설치본을 실행하고 이 프로세스가 끝나기 직전(헬퍼 정리 등)
    pub before_exit: Arc<dyn Fn() + Send + Sync>,
    /// `before_exit` 까지 돈 뒤 설치본 실행이 실패했을 때의 복구. 인자는 `install` 의 `restart`.
    /// 헬퍼가 내려가고 앱 정리가 끝난 상태라 그대로 두면 아무것도 못 하는 앱이 남는다. 실제로는 다시 시작하거나 종료한다.
    pub recover: Box<dyn Fn(bool) + Send + Sync>,
}

pub struct Updater<R: Runtime> {
    app: AppHandle<R>,
    enabled: bool,
    hooks: Hooks,
    /// 내려받는 크기 상한(시험이 작게 줄여 쓴다)
    max_download: u64,
    /// 확인·내려받기 요청 하나의 제한 시간(시험이 짧게 줄여 쓴다)
    request_timeout: Duration,
    /// 플러그인의 `on_before_exit` 훅이 돌았는가. 설치본을 실행하기 직전에 선다.
    exit_started: Arc<AtomicBool>,
    inner: Mutex<Inner>,
}

impl<R: Runtime> Updater<R> {
    pub fn new(app: AppHandle<R>, enabled: bool, hooks: Hooks) -> Arc<Self> {
        Self::with_limits(app, enabled, hooks, MAX_DOWNLOAD_BYTES, REQUEST_TIMEOUT)
    }

    fn with_limits(
        app: AppHandle<R>,
        enabled: bool,
        hooks: Hooks,
        max_download: u64,
        request_timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Updater {
            app,
            enabled,
            hooks,
            max_download,
            request_timeout,
            exit_started: Arc::new(AtomicBool::new(false)),
            inner: Mutex::new(Inner {
                state: UpdateState::Idle,
                manual: false,
                notified_version: None,
                ready: None,
            }),
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn state(&self) -> UpdateState {
        self.lock().state.clone()
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.state(), UpdateState::Ready { .. })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_state(&self, state: UpdateState) {
        self.lock().state = state;
        (self.hooks.on_change)();
    }

    /// 첫 확인과 주기 확인을 예약한다. 꺼져 있으면 아무것도 하지 않는다.
    pub fn start(self: &Arc<Self>) {
        if !self.enabled {
            return;
        }
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK).await;
            loop {
                this.run_check(false).await;
                tokio::time::sleep(CHECK_INTERVAL).await;
            }
        });
    }

    /// 트레이의 "업데이트 확인"
    pub fn check_manually(self: &Arc<Self>) {
        if !self.enabled {
            return;
        }
        let this = Arc::clone(self);
        tauri::async_runtime::spawn(async move { this.run_check(true).await });
    }

    /// 확인하고, 새 버전이면 내려받아 `Ready` 로 둔다. 이미 진행 중이거나 받아 둔 것이 있으면 아무것도 하지 않는다.
    pub async fn run_check(&self, manual: bool) {
        if !self.enabled {
            return;
        }
        {
            let mut inner = self.lock();
            if matches!(
                inner.state,
                UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Ready { .. }
            ) {
                return;
            }
            // 진행 중 검사와 같은 잠금 안에서 `Checking` 으로 바꿔, 타이머 확인과 수동 확인이 동시에 통과하지 못하게 한다
            inner.manual = manual;
            inner.state = UpdateState::Checking;
        }
        (self.hooks.on_change)();
        match self.check_and_download().await {
            Ok(None) => {
                self.set_state(UpdateState::Latest);
                if self.take_manual() {
                    (self.hooks.notify)(
                        "Peekaboo 가 최신 버전입니다",
                        &format!("현재 버전 {}", self.app.package_info().version),
                    );
                }
            }
            Ok(Some(version)) => {
                self.take_manual();
                self.set_state(UpdateState::Ready {
                    version: version.clone(),
                });
                let first_time = {
                    let mut inner = self.lock();
                    let first = inner.notified_version.as_deref() != Some(version.as_str());
                    inner.notified_version = Some(version.clone());
                    first
                };
                if first_time {
                    (self.hooks.notify)(
                        &format!("Peekaboo {version} 업데이트 준비됨"),
                        &format!(
                            "새 버전 {version} 를 받았습니다. 트레이 메뉴에서 다시 시작하여 업데이트"
                        ),
                    );
                }
            }
            Err(e) => {
                log_warn!("update failed: {e}");
                self.set_state(UpdateState::Error);
                if self.take_manual() {
                    (self.hooks.notify)(
                        "업데이트를 확인하지 못했습니다",
                        "인터넷 연결을 확인한 뒤 다시 시도해 주세요. 자세한 내용은 로그 폴더에 있습니다.",
                    );
                }
            }
        }
    }

    fn take_manual(&self) -> bool {
        std::mem::take(&mut self.lock().manual)
    }

    /// 새 버전이 있으면 내려받아 보관하고 그 버전을 돌려준다.
    async fn check_and_download(&self) -> Result<Option<String>, String> {
        let before_exit = Arc::clone(&self.hooks.before_exit);
        let exit_started = Arc::clone(&self.exit_started);
        let updater = self
            .app
            .updater_builder()
            // 기본 훅(앱 정리)을 대체하므로 이 안에서 앱 정리도 한다. 플러그인은 이 훅을 부른 뒤에 설치본을 실행하므로(updater.rs:1017-1047:
            // extract → on_before_exit → ShellExecuteW → exit(0)) 훅이 돈 뒤의 실패는 `exit_started` 로 알아 복구한다(`install`).
            .on_before_exit({
                let app = self.app.clone();
                move || {
                    exit_started.store(true, Ordering::SeqCst);
                    before_exit();
                    app.cleanup_before_exit();
                }
            })
            .timeout(self.request_timeout)
            .build()
            .map_err(|e| format!("업데이터를 만들지 못했다: {e}"))?;
        let Some(mut update) = updater
            .check()
            .await
            .map_err(|e| format!("확인 실패: {e}"))?
        else {
            return Ok(None);
        };
        // 빌더의 timeout 은 확인 요청에만 쓰인다. 내려받기 제한은 `Update` 에 직접 넣어야 한다(REQUEST_TIMEOUT 주석).
        update.timeout = Some(self.request_timeout);
        let version = update.version.clone();
        log_info!(
            "new version {version} available (current {})",
            update.current_version
        );
        // manual 은 내려받기가 끝날 때까지 둔다 — 도중에 실패하면 알려야 한다
        if self.lock().manual {
            (self.hooks.notify)(
                &format!("새 버전 {version} 을 내려받는 중입니다"),
                "끝나면 다시 알려 드립니다.",
            );
        }
        self.set_state(UpdateState::Downloading {
            version: version.clone(),
            percent: 0,
        });
        let mut received: u64 = 0;
        let mut last_percent: u8 = 0;
        // `download` 의 진행 콜백은 중단할 수 없으므로(반환값이 없다) 상한을 넘으면 표시와 알림만 올리고, 내려받기 future 를 버린다.
        // 본문이 한 번에 도착해 future 가 같은 poll 안에서 끝나면 버릴 수 없으므로, 끝난 뒤에도 표시를 다시 본다.
        let exceeded = AtomicBool::new(false);
        let too_large = tokio::sync::Notify::new();
        let max = self.max_download;
        let bytes = tokio::select! {
            result = update.download(
                |chunk, total| {
                    received += chunk as u64;
                    if download_too_large(received, total, max) {
                        exceeded.store(true, Ordering::SeqCst);
                        too_large.notify_one();
                        return;
                    }
                    let Some(total) = total.filter(|t| *t > 0) else {
                        return;
                    };
                    // 트레이 메뉴를 너무 자주 다시 만들지 않도록 10% 단위로만 갱신
                    let percent = progress_step(received, total);
                    if percent != last_percent {
                        last_percent = percent;
                        self.set_state(UpdateState::Downloading {
                            version: version.clone(),
                            percent,
                        });
                    }
                },
                || {},
            ) => result.map_err(|e| format!("내려받기 실패: {e}"))?,
            () = too_large.notified() => Vec::new(),
        };
        if exceeded.load(Ordering::SeqCst) {
            return Err(format!("설치본이 크기 상한({max} 바이트)을 넘는다"));
        }
        log_info!(
            "downloaded {version} ({} bytes), signature verified",
            bytes.len()
        );
        self.lock().ready = Some(Ready { update, bytes });
        Ok(Some(version))
    }

    /// 받아 둔 설치본을 실행한다. 성공하면 이 함수는 돌아오지 않고 프로세스가 끝난다(Windows).
    /// `restart`: 설치 뒤 앱을 다시 실행할지("다시 시작하여 업데이트"는 true, "종료"는 false).
    /// 받아 둔 것이 없거나 실패하면 false.
    pub fn install(&self, restart: bool) -> bool {
        let (update, bytes) = {
            let inner = self.lock();
            match &inner.ready {
                Some(r) => (r.update.clone(), r.bytes.clone()),
                None => return false,
            }
        };
        log_info!("installing {} (restart={restart})", update.version);
        match update.restart_after_install(restart).install(&bytes) {
            Ok(()) => true,
            Err(e) => {
                log_error!("update install failed: {e}");
                self.recover_after_failed_install(restart);
                false
            }
        }
    }

    /// 설치본을 실행하지 못했는데 플러그인의 종료 전 훅(헬퍼 정리·앱 정리)이 이미 돌았다면 복구한다.
    /// 서명 검증 실패처럼 훅 전에 난 오류는 앱이 멀쩡하므로 아무것도 하지 않는다.
    fn recover_after_failed_install(&self, restart: bool) {
        if self.exit_started.load(Ordering::SeqCst) {
            log_warn!(
                "종료 전 정리를 마친 뒤 설치에 실패했다. 앱을 {}",
                if restart {
                    "다시 시작한다"
                } else {
                    "종료한다"
                }
            );
            (self.hooks.recover)(restart);
        }
    }
}

/// 내려받는 양이 상한을 넘었는가. 응답 머리의 길이(`total`)가 이미 상한을 넘어도 본문을 받기 전에 거른다.
pub fn download_too_large(received: u64, total: Option<u64>, max: u64) -> bool {
    received > max || total.is_some_and(|t| t > max)
}

/// 받은 양을 10% 단위로 내린 진행률(0~100)
pub fn progress_step(received: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let percent = (received.min(total) * 100 / total) as u8;
    percent / 10 * 10
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // 시험 전용으로 만든 키 쌍의 공개 값이다(test/fixtures/updater-signing.json, JS 시험과 같은 파일). 개인키는 저장소에 없고
    // 어떤 릴리스에도 쓰이지 않는다. 서명은 `tauri signer sign` 으로 payload 에 만든 것이다:
    //   sigV999 = `--app-version 9.9.9` 로 서명(신뢰 주석에 `version:9.9.9`), sigNoVersion = 버전 없이 서명.
    fn signing() -> serde_json::Value {
        serde_json::from_str(include_str!("../../test/fixtures/updater-signing.json")).unwrap()
    }
    fn text(key: &str) -> String {
        signing()[key].as_str().unwrap().to_string()
    }
    fn pubkey() -> String {
        text("pubkey")
    }
    fn sig_v999() -> String {
        text("signatureV999")
    }
    fn sig_no_version() -> String {
        text("signatureNoVersion")
    }
    fn payload() -> Vec<u8> {
        text("payload").into_bytes()
    }

    /// 경로별 (상태 코드, 본문)을 돌려주는 로컬 HTTP 서버(127.0.0.1 에만 묶는다). 포트를 돌려준다.
    /// 라우트는 포트를 알아야 만들 수 있어(latest.json 의 url) 포트를 받는 함수로 넘긴다.
    fn serve(routes: impl FnOnce(u16) -> Vec<(&'static str, u16, Vec<u8>)>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes = routes(port);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = routes
                    .iter()
                    .find(|(p, _, _)| *p == path)
                    .map_or((404, Vec::new()), |(_, s, b)| (*s, b.clone()));
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        port
    }

    /// latest.json(`/latest.json`)과 설치본(`/installer.exe`)을 내는 서버
    fn serve_release(version: &str, signature: &str, payload: &[u8]) -> u16 {
        let (version, signature, payload) =
            (version.to_string(), signature.to_string(), payload.to_vec());
        serve(move |port| {
            let feed = serde_json::json!({
                "version": version,
                "notes": "시험",
                "pub_date": "2026-11-16T00:00:00Z",
                "platforms": {
                    "windows-x86_64": {
                        "signature": signature,
                        "url": format!("http://127.0.0.1:{port}/installer.exe"),
                    }
                }
            })
            .to_string()
            .into_bytes();
            vec![
                ("/latest.json", 200, feed),
                ("/installer.exe", 200, payload),
            ]
        })
    }

    /// 설치본 응답이 정상이 아닌 서버의 동작
    #[derive(Clone, Copy)]
    enum Misbehave {
        /// 머리(Content-Length 1000)만 보내고 연결을 쥔 채 멈춘다(제한 시간 시험)
        Stall,
        /// 머리(Content-Length 1 TiB)를 보내고 클라이언트가 끊을 때까지 본문을 계속 쏟는다(받는 도중 중단 시험).
        /// 중단하지 않으면 제한 시간(10분)까지 끝나지 않으므로 중단 경로가 없으면 시험이 멈춘다.
        Endless,
    }

    /// latest.json 은 정상으로 내고, 설치본(`/installer.exe`)은 `how` 대로 이상하게 구는 서버
    fn serve_misbehaving(version: &str, signature: &str, how: Misbehave) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let feed = serde_json::json!({
            "version": version,
            "notes": "시험",
            "pub_date": "2026-11-16T00:00:00Z",
            "platforms": {
                "windows-x86_64": {
                    "signature": signature,
                    "url": format!("http://127.0.0.1:{port}/installer.exe"),
                }
            }
        })
        .to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let feed = feed.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = request.split_whitespace().nth(1).unwrap_or("/");
                    if path == "/latest.json" {
                        let head = format!(
                            "HTTP/1.1 200 X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            feed.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(feed.as_bytes());
                        return;
                    }
                    match how {
                        Misbehave::Stall => {
                            let _ = stream.write_all(
                                b"HTTP/1.1 200 X\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n",
                            );
                            std::thread::sleep(Duration::from_secs(20));
                        }
                        Misbehave::Endless => {
                            let head = b"HTTP/1.1 200 X\r\nContent-Length: 1099511627776\r\nConnection: close\r\n\r\n";
                            if stream.write_all(head).is_err() {
                                return;
                            }
                            let block = [0u8; 64 * 1024];
                            // 클라이언트가 끊으면 쓰기가 실패해 끝난다
                            while stream.write_all(&block).is_ok() {}
                        }
                    }
                });
            }
        });
        port
    }

    #[derive(Default)]
    struct Calls {
        notices: Mutex<Vec<(String, String)>>,
        changes: Mutex<usize>,
        /// 설치 실패 복구가 불린 횟수와 그때의 restart 값
        recovered: Mutex<Vec<bool>>,
    }

    /// 실제 플러그인을 목 런타임에 올리고 엔드포인트를 로컬 서버로 돌린 업데이터와 앱의 현재 버전
    fn updater_for(
        port: u16,
        pubkey: &str,
        calls: &Arc<Calls>,
    ) -> (Arc<Updater<tauri::test::MockRuntime>>, String) {
        updater_with_limits(port, pubkey, calls, MAX_DOWNLOAD_BYTES, REQUEST_TIMEOUT)
    }

    fn updater_with_limit(
        port: u16,
        pubkey: &str,
        calls: &Arc<Calls>,
        max_download: u64,
    ) -> (Arc<Updater<tauri::test::MockRuntime>>, String) {
        updater_with_limits(port, pubkey, calls, max_download, REQUEST_TIMEOUT)
    }

    /// 플러그인 설정은 scripts/updater-config.mjs 가 만드는 것과 같은 기준 파일(test/fixtures/updater-plugin.json)에서 읽는다.
    /// JS 시험(test/update-feeds.test.ts)이 buildUpdaterConfig 의 결과와 이 파일이 같은지 확인하므로 두 모양이 어긋나면 한쪽 시험이 깨진다.
    /// 공개키와 엔드포인트만 자리표시자를 바꿔 넣는다. `requireSignedVersion: true` 도 그 파일에 들어 있다.
    fn plugin_config(port: u16, pubkey: &str) -> serde_json::Value {
        let mut config: serde_json::Value =
            serde_json::from_str(include_str!("../../test/fixtures/updater-plugin.json")).unwrap();
        let object = config.as_object_mut().unwrap();
        object.remove("_comment");
        assert_eq!(object["pubkey"], "PUBKEY");
        object.insert("pubkey".into(), pubkey.into());
        object.insert(
            "endpoints".into(),
            serde_json::json!([format!("http://127.0.0.1:{port}/latest.json")]),
        );
        config
    }

    fn updater_with_limits(
        port: u16,
        pubkey: &str,
        calls: &Arc<Calls>,
        max_download: u64,
        request_timeout: Duration,
    ) -> (Arc<Updater<tauri::test::MockRuntime>>, String) {
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context
            .config_mut()
            .plugins
            .0
            .insert("updater".into(), plugin_config(port, pubkey));
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        let current = app.package_info().version.to_string();
        let notify_calls = Arc::clone(calls);
        let change_calls = Arc::clone(calls);
        let recover_calls = Arc::clone(calls);
        let updater = Updater::with_limits(
            app.handle().clone(),
            true,
            Hooks {
                notify: Box::new(move |t, b| {
                    notify_calls
                        .notices
                        .lock()
                        .unwrap()
                        .push((t.to_string(), b.to_string()));
                }),
                on_change: Box::new(move || *change_calls.changes.lock().unwrap() += 1),
                before_exit: Arc::new(|| {}),
                recover: Box::new(move |restart| {
                    recover_calls.recovered.lock().unwrap().push(restart)
                }),
            },
            max_download,
            request_timeout,
        );
        (updater, current)
    }

    #[test]
    fn 켜지는_조건() {
        assert_eq!(disabled_reason(true, "1.0.0", "k"), Some("디버그 빌드"));
        assert!(disabled_reason(false, "0.0.0", "k").is_some());
        assert!(disabled_reason(false, "0.0.0-ci.3", "k").is_some());
        assert_eq!(
            disabled_reason(false, "1.0.0", "  "),
            Some("서명 공개키가 없음")
        );
        assert_eq!(disabled_reason(false, "0.0.1", "k"), None);
        assert_eq!(disabled_reason(false, "1.0.0-beta.1", "k"), None);
    }

    #[test]
    fn 진행률은_10퍼센트_단위로_내린다() {
        assert_eq!(progress_step(0, 100), 0);
        assert_eq!(progress_step(9, 100), 0);
        assert_eq!(progress_step(10, 100), 10);
        assert_eq!(progress_step(99, 100), 90);
        assert_eq!(progress_step(100, 100), 100);
        assert_eq!(progress_step(500, 100), 100);
        assert_eq!(progress_step(1, 0), 0);
    }

    #[tokio::test]
    async fn 새_버전을_받으면_준비됨_알림은_한_번만() {
        let port = serve_release("9.9.9", &sig_v999(), &payload());
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, &pubkey(), &calls);

        updater.run_check(false).await;
        assert_eq!(
            updater.state(),
            UpdateState::Ready {
                version: "9.9.9".into()
            }
        );
        assert!(updater.is_ready());
        {
            let notices = calls.notices.lock().unwrap();
            assert_eq!(notices.len(), 1);
            assert!(notices[0].1.contains("새 버전 9.9.9 를 받았습니다"));
            assert!(notices[0].1.contains("다시 시작하여 업데이트"));
        }
        // 받아 둔 것이 있으면 다시 확인하지 않는다(수동 확인도 알림을 더하지 않는다)
        updater.run_check(false).await;
        updater.run_check(true).await;
        assert_eq!(calls.notices.lock().unwrap().len(), 1);
        assert!(*calls.changes.lock().unwrap() >= 3);
    }

    #[tokio::test]
    async fn 같은_버전이면_최신_수동_확인에만_알린다() {
        // 현재 버전은 앱에서 알아내야 하므로 서버를 띄우기 전에 한 번 만들어 본다
        let (_, current) = updater_for(1, &pubkey(), &Arc::new(Calls::default()));
        let port = serve_release(&current, &sig_v999(), &payload());
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, &pubkey(), &calls);

        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Latest);
        assert!(calls.notices.lock().unwrap().is_empty());

        updater.run_check(true).await;
        assert_eq!(updater.state(), UpdateState::Latest);
        let notices = calls.notices.lock().unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].0, "Peekaboo 가 최신 버전입니다");
        assert!(!updater.is_ready());
    }

    #[tokio::test]
    async fn 서명이_맞지_않는_설치본은_받아도_준비되지_않는다() {
        let port = serve_release("9.9.9", &sig_v999(), b"tampered-payload");
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, &pubkey(), &calls);

        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(!updater.is_ready());
        assert!(!updater.install(true));
        // 자동 확인의 실패는 알리지 않는다
        assert!(calls.notices.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn 공개키가_맞지_않으면_받은_설치본을_거부한다() {
        let port = serve_release("9.9.9", &sig_v999(), &payload());
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, "AAAA", &calls);

        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(!updater.is_ready());
    }

    #[tokio::test]
    async fn 서버가_실패하면_수동_확인에만_오류를_알린다() {
        let port = serve(|_| vec![("/latest.json", 500, b"oops".to_vec())]);
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, &pubkey(), &calls);

        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(calls.notices.lock().unwrap().is_empty());

        // 오류 뒤에는 다시 확인할 수 있다
        updater.run_check(true).await;
        assert_eq!(updater.state(), UpdateState::Error);
        let notices = calls.notices.lock().unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].0, "업데이트를 확인하지 못했습니다");
    }

    #[tokio::test]
    async fn 꺼진_업데이터는_아무것도_하지_않는다() {
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(1, &pubkey(), &calls);
        let off = Updater::new(
            updater.app.clone(),
            false,
            Hooks {
                notify: Box::new(|_, _| panic!("알림을 보내면 안 된다")),
                on_change: Box::new(|| panic!("상태가 바뀌면 안 된다")),
                before_exit: Arc::new(|| {}),
                recover: Box::new(|_| panic!("복구하면 안 된다")),
            },
        );
        off.run_check(true).await;
        assert_eq!(off.state(), UpdateState::Idle);
        assert!(!off.enabled());
    }

    #[tokio::test]
    async fn 서명이_덮는_버전과_피드_버전이_다르면_거부한다() {
        // 9.9.9 로 서명한 설치본을 피드가 9.9.8(더 낮거나 다른 번호)로 광고한다 — 피드 바꿔치기(다운그레이드)
        let port = serve_release("9.9.8", &sig_v999(), &payload());
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, &pubkey(), &calls);

        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(!updater.is_ready());
    }

    #[tokio::test]
    async fn 버전이_없는_서명은_거부한다() {
        // 서명 자체는 payload 에 유효하지만 신뢰 주석에 version 이 없다 — requireSignedVersion 이 막는다
        let port = serve_release("9.9.9", &sig_no_version(), &payload());
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, &pubkey(), &calls);

        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(!updater.is_ready());
    }

    #[tokio::test]
    async fn 크기_상한을_넘는_설치본은_받지_않는다() {
        // 본문이 한 번에 도착하면 내려받기 future 를 버릴 수 없어 끝난 뒤 표시로 거른다(도중 중단은 아래 멈춘 서버 시험)
        let port = serve_release("9.9.9", &sig_v999(), &payload());
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_with_limit(port, &pubkey(), &calls, 8);
        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(!updater.is_ready());

        // 상한 안이면 같은 설치본을 받는다
        let (updater, _) = updater_with_limit(port, &pubkey(), &calls, payload().len() as u64);
        updater.run_check(false).await;
        assert_eq!(
            updater.state(),
            UpdateState::Ready {
                version: "9.9.9".into()
            }
        );
    }

    #[test]
    fn 상한_판정() {
        assert!(!download_too_large(10, Some(10), 10));
        assert!(!download_too_large(10, None, 10));
        assert!(download_too_large(11, None, 10));
        // 길이를 알리는 응답은 본문을 받기 전에 거른다
        assert!(download_too_large(0, Some(11), 10));
    }

    #[test]
    fn 요청_제한과_상한은_정해진_값이다() {
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(600));
        assert_eq!(MAX_DOWNLOAD_BYTES, 64 * 1024 * 1024);
    }

    /// 서버가 끝없이 본문을 쏟아도, 받은 양이 상한(8)을 넘으면 내려받기 future 를 버리고 끝낸다. 중단 경로가 없으면 제한 시간(10분)까지 돈다.
    #[tokio::test]
    async fn 끝없는_본문은_크기_상한에서_받는_도중_중단한다() {
        let port = serve_misbehaving("9.9.9", &sig_v999(), Misbehave::Endless);
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_with_limit(port, &pubkey(), &calls, 8);
        let started = std::time::Instant::now();
        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(!updater.is_ready());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// 내려받기 제한 시간이 `Update.timeout` 으로 실제로 적용된다(플러그인 2.14.1 은 `check()` 결과에 넣어 주지 않는다).
    /// 서버가 머리만 보내고 멈추면 제한 시간(0.5초)에 끊겨 Error 가 된다. 대입을 빼면 서버가 연결을 닫는 20초까지 기다린다.
    #[tokio::test]
    async fn 멈춘_서버는_요청_제한_시간에_끊긴다() {
        let port = serve_misbehaving("9.9.9", &sig_v999(), Misbehave::Stall);
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_with_limits(
            port,
            &pubkey(),
            &calls,
            MAX_DOWNLOAD_BYTES,
            Duration::from_millis(500),
        );
        let started = std::time::Instant::now();
        updater.run_check(false).await;
        assert_eq!(updater.state(), UpdateState::Error);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn 설치_실패는_종료_전_정리가_돈_뒤에만_복구한다() {
        let port = serve_release("9.9.9", &sig_v999(), &payload());
        let calls = Arc::new(Calls::default());
        let (updater, _) = updater_for(port, &pubkey(), &calls);
        updater.run_check(false).await;
        assert!(updater.is_ready());
        // 받아 둔 설치본을 깨뜨려 서명 검증에서 막히게 한다. 설치기를 실제로 실행하지 않고 오류 경로만 본다
        // (실제 설치는 ShellExecute 뒤에 프로세스를 끝낸다).
        updater.lock().ready.as_mut().unwrap().bytes = b"corrupted".to_vec();

        // 종료 전 훅이 돌기 전에 난 실패(여기서는 서명 검증)는 앱이 멀쩡하므로 복구하지 않는다
        assert!(!updater.install(true));
        assert!(calls.recovered.lock().unwrap().is_empty());

        // 훅이 돈 뒤의 실패는 복구한다: 다시 시작(true)·종료(false)
        updater.exit_started.store(true, Ordering::SeqCst);
        assert!(!updater.install(true));
        assert!(!updater.install(false));
        assert_eq!(*calls.recovered.lock().unwrap(), vec![true, false]);
    }
}
