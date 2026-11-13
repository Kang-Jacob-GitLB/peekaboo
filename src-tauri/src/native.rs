// Windows 미리보기 핸들러 요청을 헬퍼로 중계한다. Electron 의 src/main/native-preview.ts 와 같은 의미다.
// 핸들러는 헬퍼(전용 STA 스레드)가 띄우고, 핸들러 창은 미리보기 창 HWND 의 자식으로 붙는다.
// 동시에 하나만 연다 — 새로 열면 헬퍼가 이전 것을 Unload 한다.
//
// 렌더러가 부르는 커맨드(native_open·native_resize·native_close)는 commands.rs 에 있고, 여기는 id·대기 관리만 한다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;

use crate::helper::{HostMessage, NativeStatus, PixelRect};

/// 헬퍼가 응답하지 않을 때 렌더러가 대체 렌더러로 넘어가도록 하는 상한
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(8);

/// `PeekabooApi.nativePreview.open` 의 결과
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NativePreviewResult {
    /// resize/close 에 쓰는 식별자
    pub handle: i32,
    pub status: NativeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl NativePreviewResult {
    pub fn failed(handle: i32, message: &str) -> Self {
        NativePreviewResult {
            handle,
            status: NativeStatus::Failed,
            message: Some(message.to_owned()),
        }
    }
}

/// 헬퍼에 메시지를 보낸다. 헬퍼가 떠 있지 않으면 false.
type SendFn = Arc<dyn Fn(&HostMessage) -> bool + Send + Sync>;

#[derive(Default)]
struct Inner {
    next_id: i32,
    /// 마지막으로 연 요청. resize/close 는 이것만 받는다.
    current: Option<i32>,
    pending: HashMap<i32, oneshot::Sender<NativePreviewResult>>,
}

pub struct NativeBridge {
    send: SendFn,
    timeout: Duration,
    inner: Mutex<Inner>,
}

impl NativeBridge {
    pub fn new(send: impl Fn(&HostMessage) -> bool + Send + Sync + 'static) -> Self {
        Self::with_timeout(send, OPEN_TIMEOUT)
    }

    pub fn with_timeout(
        send: impl Fn(&HostMessage) -> bool + Send + Sync + 'static,
        timeout: Duration,
    ) -> Self {
        NativeBridge {
            send: Arc::new(send),
            timeout,
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 핸들러를 연다. 응답(`on_result`)·타임아웃·`close_all` 중 먼저 오는 것이 결과가 된다.
    pub async fn open(
        &self,
        path: &str,
        hwnd: i64,
        rect: PixelRect,
        dark: bool,
    ) -> NativePreviewResult {
        let (tx, rx) = oneshot::channel();
        let id = {
            let mut s = self.lock();
            s.next_id = s.next_id.checked_add(1).unwrap_or(1);
            let id = s.next_id;
            s.current = Some(id);
            // 응답이 보내는 쪽보다 먼저 올 수 있으니 보내기 전에 등록한다
            s.pending.insert(id, tx);
            id
        };
        let msg = HostMessage::NativePreviewOpen {
            id,
            path: path.to_owned(),
            hwnd,
            rect,
            dark,
            // WebView2 는 새 자식 창(컨테이너)이 웹뷰 자식 창 아래에 깔린다(docs/ipc.md)
            raise: true,
        };
        // 헬퍼가 죽었거나 재시작 중이면 타임아웃까지 기다리지 않고 바로 자체 렌더러로 넘긴다
        if !(self.send)(&msg) {
            let mut s = self.lock();
            s.pending.remove(&id);
            if s.current == Some(id) {
                s.current = None;
            }
            eprintln!("[peekaboo] native #{id}: 헬퍼가 떠 있지 않아 자체 렌더러로 넘긴다");
            return NativePreviewResult::failed(id, "helper not running");
        }
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(result)) => result,
            // 보내는 쪽이 사라짐: 정리 중
            Ok(Err(_)) => NativePreviewResult::failed(id, "cancelled"),
            Err(_) => {
                eprintln!("[peekaboo] native open #{id} 시간 초과");
                self.lock().pending.remove(&id);
                self.close(id);
                NativePreviewResult::failed(id, "timeout")
            }
        }
    }

    pub fn resize(&self, id: i32, rect: PixelRect) {
        if self.lock().current == Some(id) {
            (self.send)(&HostMessage::NativePreviewResize { id, rect });
        }
    }

    pub fn close(&self, id: i32) {
        {
            let mut s = self.lock();
            if s.current != Some(id) {
                return;
            }
            s.current = None;
        }
        (self.send)(&HostMessage::NativePreviewClose { id });
    }

    /// 미리보기 창이 닫히거나 헬퍼가 재시작됐을 때. 열린 것은 닫고, 대기 중인 요청은 failed 로 끝낸다.
    pub fn close_all(&self, reason: &str) {
        let (current, pending) = {
            let mut s = self.lock();
            (s.current, std::mem::take(&mut s.pending))
        };
        if let Some(id) = current {
            self.close(id);
        }
        for (id, tx) in pending {
            let _ = tx.send(NativePreviewResult::failed(id, reason));
        }
    }

    /// 헬퍼의 nativePreview 응답
    pub fn on_result(&self, id: i32, status: NativeStatus, message: Option<String>) {
        let Some(tx) = self.lock().pending.remove(&id) else {
            return;
        };
        if status != NativeStatus::Shown {
            eprintln!(
                "[peekaboo] native #{id} {status:?}{}",
                message
                    .as_deref()
                    .map(|m| format!(": {m}"))
                    .unwrap_or_default()
            );
        }
        let _ = tx.send(NativePreviewResult {
            handle: id,
            status,
            message,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> PixelRect {
        PixelRect {
            x: 0.0,
            y: 48.0,
            width: 100.0,
            height: 100.0,
        }
    }

    /// 보낸 메시지를 모으는 브리지. `alive` 가 false 면 헬퍼가 없는 것처럼 보내기가 실패한다.
    fn bridge(alive: bool, timeout: Duration) -> (Arc<NativeBridge>, Arc<Mutex<Vec<HostMessage>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&sent);
        let b = NativeBridge::with_timeout(
            move |m| {
                if alive {
                    log.lock().unwrap().push(m.clone());
                }
                alive
            },
            timeout,
        );
        (Arc::new(b), sent)
    }

    #[tokio::test]
    async fn 헬퍼가_없으면_기다리지_않고_failed() {
        let (b, sent) = bridge(false, Duration::from_secs(60));
        let r = b.open("C:\\a.docx", 1, rect(), false).await;
        assert_eq!(r.status, NativeStatus::Failed);
        assert_eq!(r.message.as_deref(), Some("helper not running"));
        assert!(sent.lock().unwrap().is_empty());
        // 현재 요청이 남지 않아 resize 도 보내지 않는다
        b.resize(r.handle, rect());
    }

    #[tokio::test]
    async fn 응답이_오면_그_결과를_돌려준다() {
        let (b, sent) = bridge(true, Duration::from_secs(60));
        let b2 = Arc::clone(&b);
        let task = tokio::spawn(async move { b2.open("C:\\a.docx", 9, rect(), true).await });
        while sent.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            sent.lock().unwrap()[0],
            HostMessage::NativePreviewOpen {
                id: 1,
                hwnd: 9,
                dark: true,
                ..
            }
        ));
        b.on_result(1, NativeStatus::Shown, None);
        let r = task.await.unwrap();
        assert_eq!(
            r,
            NativePreviewResult {
                handle: 1,
                status: NativeStatus::Shown,
                message: None
            }
        );
        // 열린 요청에만 resize·close 가 간다
        b.resize(1, rect());
        b.resize(2, rect());
        b.close(2);
        b.close(1);
        b.close(1);
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 3);
        assert!(matches!(
            sent[1],
            HostMessage::NativePreviewResize { id: 1, .. }
        ));
        assert_eq!(sent[2], HostMessage::NativePreviewClose { id: 1 });
    }

    #[tokio::test]
    async fn 응답이_없으면_시간_초과로_failed_이고_닫는다() {
        let (b, sent) = bridge(true, Duration::from_millis(30));
        let r = b.open("C:\\a.docx", 1, rect(), false).await;
        assert_eq!(r.status, NativeStatus::Failed);
        assert_eq!(r.message.as_deref(), Some("timeout"));
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1], HostMessage::NativePreviewClose { id: 1 });
    }

    #[tokio::test]
    async fn close_all_은_열린_것을_닫고_대기를_failed_로_끝낸다() {
        let (b, sent) = bridge(true, Duration::from_secs(60));
        let b2 = Arc::clone(&b);
        let task = tokio::spawn(async move { b2.open("C:\\a.docx", 1, rect(), false).await });
        while sent.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
        b.close_all("hidden");
        let r = task.await.unwrap();
        assert_eq!(r, NativePreviewResult::failed(1, "hidden"));
        assert_eq!(
            sent.lock().unwrap().last(),
            Some(&HostMessage::NativePreviewClose { id: 1 })
        );
        // 늦게 온 응답은 무시한다
        b.on_result(1, NativeStatus::Shown, None);
    }

    #[tokio::test]
    async fn 새로_열면_이전_요청은_현재가_아니다() {
        let (b, sent) = bridge(true, Duration::from_millis(30));
        let _ = b.open("a", 1, rect(), false).await; // 시간 초과 뒤 닫힘
        let _ = b.open("b", 1, rect(), false).await;
        let ids: Vec<i32> = sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| match m {
                HostMessage::NativePreviewOpen { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(ids, [1, 2]);
    }

    #[test]
    fn 결과는_camel_case_json_으로_직렬화된다() {
        let v = serde_json::to_value(NativePreviewResult {
            handle: 3,
            status: NativeStatus::Unavailable,
            message: Some("internet-zone".into()),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"handle":3,"status":"unavailable","message":"internet-zone"})
        );
        let v = serde_json::to_value(NativePreviewResult {
            handle: 0,
            status: NativeStatus::Shown,
            message: None,
        })
        .unwrap();
        assert_eq!(v, serde_json::json!({"handle":0,"status":"shown"}));
    }
}
