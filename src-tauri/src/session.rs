// 헬퍼 메시지 → 호스트 동작 매핑. Electron 의 src/main/index.ts 의 `helper.on('message')` 와 openPreview·
// closePreview·showSelection 에 해당한다.
// 창·프로세스를 건드리지 않는 순수 로직이라 cargo test 로 검증하고, 돌려준 `Action` 을 실제로 실행하는 것은 controller.rs 다.

use std::collections::HashSet;

use crate::helper::{HelperMessage, HostMessage, KeyName, NativeStatus};

/// 메시지를 해석할 때 필요한 지금의 호스트 상태
pub struct Ctx<'a> {
    /// 미리보기가 보이는 중이거나 띄우는 중
    pub preview_open: bool,
    /// 보이는 창에 텍스트가 선택돼 있다(헬퍼가 다시 뜰 때 알려야 한다)
    pub text_selected: bool,
    /// 단축키 설정(`config.enabled`)
    pub enabled: bool,
    /// 이 앱의 PID
    pub app_pid: u32,
    /// 앱 + WebView2 브라우저 프로세스(헬퍼에 보내는 `hostPids`). `foreground` 의 "우리 창" 판정에도 쓴다.
    pub host_pids: &'a [u32],
}

impl Ctx<'_> {
    /// 헬퍼가 (재)시작되면 다시 알려야 하는 설정
    pub fn config(&self) -> HostMessage {
        HostMessage::Config {
            enabled: self.enabled,
            host_pid: self.app_pid,
            host_pids: self.host_pids.to_vec(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Send(HostMessage),
    /// 파일을 미리보기로 띄운다(창이 이미 열려 있으면 내용만 바꾼다)
    Show {
        path: String,
        selection_count: usize,
    },
    /// 미리보기를 닫는다(대기 중인 열기 요청도 취소). 헬퍼에 `state{previewOpen:false}` 도 항상 알린다.
    Close,
    /// Ctrl+C: 렌더러에 선택한 텍스트를 묻는다(요청 번호는 PreviewState 가 만든다)
    RequestCopy,
    /// 클릭하면 포커스를 받는 창으로 할지(다른 앱이 포그라운드인 동안만 true)
    SetActivatable(bool),
    /// 네이티브 미리보기 요청의 응답
    NativeResult {
        id: i32,
        status: NativeStatus,
        message: Option<String>,
    },
    /// 헬퍼가 다시 떠서 이전 헬퍼의 네이티브 미리보기 요청이 사라졌다
    NativeCloseAll(&'static str),
    Log(String),
}

/// 헬퍼에 알리는 `state` 메시지. 값은 호스트의 현재 상태(PreviewState::is_open)다.
/// 열기 요청을 만들 때 `previewOpen: true` 를 먼저 보내므로, 표시가 끝나지 않고 끝난 경로(닫기·큐에서 폐기·포기·실패)에서는
/// 상태를 다시 읽어 이 메시지로 false 를 보내야 헬퍼의 Esc 삼키기·선택 감시가 켜진 채 남지 않는다(controller.rs).
pub fn state_message(preview_open: bool) -> HostMessage {
    HostMessage::State { preview_open }
}

#[derive(Default)]
pub struct Session {
    next_id: i32,
    /// getSelection 요청 id. 응답이 오면 미리보기를 띄우고, 그 전에 닫히면 버린다.
    pending_open: HashSet<i32>,
}

impl Session {
    /// 닫기 요청: 응답을 기다리던 열기 요청을 모두 취소한다.
    pub fn cancel_pending(&mut self) {
        self.pending_open.clear();
    }

    fn open_preview(&mut self) -> Vec<Action> {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.pending_open.insert(id);
        vec![Action::Send(HostMessage::GetSelection { id })]
    }

    fn show_selection(paths: &[String], focused: Option<&str>, preview_open: bool) -> Vec<Action> {
        // 포커스 항목이 선택에 있으면 그것, 아니면 첫 항목, 그것도 없으면 포커스만 있는 항목
        let path = match focused {
            Some(f) if paths.iter().any(|p| p == f) => Some(f),
            _ => paths.first().map(String::as_str).or(focused),
        };
        let Some(path) = path else {
            return Vec::new(); // 선택 없음
        };
        let mut out = Vec::new();
        // Esc 삼키기와 선택 변경 감시가 이 상태에 의존하므로 창을 띄우기 전에 알린다
        if !preview_open {
            out.push(Action::Send(HostMessage::State { preview_open: true }));
        }
        out.push(Action::Show {
            path: path.to_owned(),
            selection_count: paths.len(),
        });
        out
    }

    pub fn on_message(&mut self, msg: HelperMessage, ctx: &Ctx) -> Vec<Action> {
        match msg {
            HelperMessage::Ready { .. } => vec![
                // 이전 헬퍼가 띄운 미리보기 핸들러는 사라졌으므로 대기 중인 요청을 끝낸다
                Action::NativeCloseAll("helper restarted"),
                // 현재 설정·상태를 다시 알린다
                Action::Send(ctx.config()),
                Action::Send(HostMessage::State {
                    preview_open: ctx.preview_open,
                }),
            ]
            .into_iter()
            // 헬퍼는 다시 뜨면 선택이 없다고 보므로, 선택이 남아 있으면 알린다
            .chain(
                ctx.text_selected
                    .then_some(Action::Send(HostMessage::TextSelection { selected: true })),
            )
            .collect(),
            HelperMessage::Key { key, .. } => match key {
                // 여는 중(getSelection 응답 대기)에 다시 Space 를 누르면 취소로 본다
                KeyName::Space if ctx.preview_open || !self.pending_open.is_empty() => {
                    vec![Action::Close]
                }
                KeyName::Space => self.open_preview(),
                KeyName::Esc | KeyName::Enter => vec![Action::Close],
                // 헬퍼는 미리보기에 텍스트 선택이 있다고 알려 준 경우에만 복사 키를 보낸다
                KeyName::Copy => vec![Action::RequestCopy],
                // 방향키: 탐색기가 선택을 옮기고, 헬퍼가 selection 알림을 보낸다
                _ => Vec::new(),
            },
            HelperMessage::Selection {
                id: Some(id),
                paths,
                focused,
            } => {
                if !self.pending_open.remove(&id) {
                    return Vec::new(); // 이미 닫힘/취소됨
                }
                Self::show_selection(&paths, focused.as_deref(), ctx.preview_open)
            }
            HelperMessage::Selection {
                id: None,
                paths,
                focused,
            } => {
                if ctx.preview_open {
                    Self::show_selection(&paths, focused.as_deref(), true)
                } else {
                    Vec::new()
                }
            }
            HelperMessage::NativePreview {
                id,
                status,
                message,
            } => vec![Action::NativeResult {
                id,
                status,
                message,
            }],
            HelperMessage::Foreground { pid, shell } => {
                // 다른 앱으로 전환해도 닫지 않는다(Quick Look 처럼 떠 있게 둔다). 이 앱 자신의 창(미리보기, WebView2 의
                // `<select>` 팝업 같은 브라우저 프로세스의 창)으로 바뀐 것은 전환이 아니다.
                if ctx.host_pids.contains(&pid) {
                    return Vec::new();
                }
                // 다른 앱이 포그라운드인 동안은 미리보기를 클릭하면 포커스가 가게 해, 그 상태의 Space/Esc 로도 닫을 수 있게 한다.
                // 셸(탐색기 등)이 포그라운드면 클릭해도 탐색기 포커스가 남아야 한다. 값이 바뀔 때만 적용하는 것은 실행하는 쪽이 한다.
                let mut actions = vec![Action::SetActivatable(!shell)];
                if !shell && ctx.preview_open {
                    actions.push(Action::Log(format!(
                        "foreground moved to pid {pid} (preview kept open)"
                    )));
                }
                actions
            }
            HelperMessage::Error { id, code, message } => {
                if let Some(id) = id {
                    self.pending_open.remove(&id);
                }
                vec![Action::Log(format!("helper error {code}: {message}"))]
            }
            // pong 은 감시 루프가 먹는다
            HelperMessage::Pong { .. } | HelperMessage::Unknown => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::PreviewState;

    fn ctx(preview_open: bool) -> Ctx<'static> {
        Ctx {
            preview_open,
            text_selected: false,
            enabled: true,
            app_pid: 100,
            host_pids: &[100, 200],
        }
    }

    fn key(k: KeyName) -> HelperMessage {
        HelperMessage::Key {
            key: k,
            consumed: true,
        }
    }

    fn selection(id: Option<i32>, paths: &[&str], focused: Option<&str>) -> HelperMessage {
        HelperMessage::Selection {
            id,
            paths: paths.iter().map(|s| (*s).to_owned()).collect(),
            focused: focused.map(str::to_owned),
        }
    }

    #[test]
    fn ready_는_설정과_상태를_다시_보낸다() {
        let mut s = Session::default();
        let got = s.on_message(
            HelperMessage::Ready {
                version: "1".into(),
                pid: 5,
            },
            &Ctx {
                enabled: false,
                ..ctx(true)
            },
        );
        assert_eq!(
            got,
            vec![
                Action::NativeCloseAll("helper restarted"),
                Action::Send(HostMessage::Config {
                    enabled: false,
                    host_pid: 100,
                    host_pids: vec![100, 200]
                }),
                Action::Send(HostMessage::State { preview_open: true }),
            ]
        );
    }

    #[test]
    fn space_는_닫혀_있으면_선택_조회_열려_있으면_닫기() {
        let mut s = Session::default();
        let got = s.on_message(key(KeyName::Space), &ctx(false));
        assert_eq!(got, vec![Action::Send(HostMessage::GetSelection { id: 1 })]);
        // 응답을 기다리는 중에 다시 누르면 취소
        assert_eq!(
            s.on_message(key(KeyName::Space), &ctx(false)),
            vec![Action::Close]
        );
        // 열려 있을 때
        let mut s = Session::default();
        assert_eq!(
            s.on_message(key(KeyName::Space), &ctx(true)),
            vec![Action::Close]
        );
    }

    #[test]
    fn esc_와_enter_는_닫는다_방향키는_아무것도_안_한다() {
        let mut s = Session::default();
        for k in [KeyName::Esc, KeyName::Enter] {
            assert_eq!(s.on_message(key(k), &ctx(true)), vec![Action::Close]);
        }
        for k in [KeyName::Up, KeyName::Down, KeyName::Left, KeyName::Right] {
            assert!(s.on_message(key(k), &ctx(true)).is_empty());
        }
    }

    #[test]
    fn 선택_응답이_오면_state_를_먼저_알리고_띄운다() {
        let mut s = Session::default();
        s.on_message(key(KeyName::Space), &ctx(false));
        let got = s.on_message(selection(Some(1), &["a", "b"], Some("b")), &ctx(false));
        assert_eq!(
            got,
            vec![
                Action::Send(HostMessage::State { preview_open: true }),
                Action::Show {
                    path: "b".into(),
                    selection_count: 2
                },
            ]
        );
    }

    #[test]
    fn 이미_열려_있으면_state_를_다시_보내지_않는다() {
        let mut s = Session::default();
        let got = s.on_message(selection(None, &["a"], Some("a")), &ctx(true));
        assert_eq!(
            got,
            vec![Action::Show {
                path: "a".into(),
                selection_count: 1
            }]
        );
    }

    #[test]
    fn 닫힌_뒤_온_선택_응답과_알림은_버린다() {
        let mut s = Session::default();
        s.on_message(key(KeyName::Space), &ctx(false));
        s.cancel_pending();
        assert!(s
            .on_message(selection(Some(1), &["a"], Some("a")), &ctx(false))
            .is_empty());
        // id 없는 알림은 열려 있을 때만
        assert!(s
            .on_message(selection(None, &["a"], Some("a")), &ctx(false))
            .is_empty());
        // 모르는 id
        assert!(s
            .on_message(selection(Some(99), &["a"], Some("a")), &ctx(false))
            .is_empty());
    }

    #[test]
    fn 포커스가_선택에_없으면_첫_항목_선택이_비면_포커스() {
        assert_eq!(
            Session::show_selection(&["a".into(), "b".into()], Some("zzz"), true),
            vec![Action::Show {
                path: "a".into(),
                selection_count: 2
            }]
        );
        assert_eq!(
            Session::show_selection(&[], Some("f"), true),
            vec![Action::Show {
                path: "f".into(),
                selection_count: 0
            }]
        );
        assert!(Session::show_selection(&[], None, true).is_empty());
    }

    #[test]
    fn 오류_응답은_대기_중인_열기를_지운다() {
        let mut s = Session::default();
        s.on_message(key(KeyName::Space), &ctx(false));
        s.on_message(
            HelperMessage::Error {
                id: Some(1),
                code: "NO_VIEW".into(),
                message: "m".into(),
            },
            &ctx(false),
        );
        // 대기가 비었으므로 다음 Space 는 다시 조회
        assert_eq!(
            s.on_message(key(KeyName::Space), &ctx(false)),
            vec![Action::Send(HostMessage::GetSelection { id: 2 })]
        );
    }

    #[test]
    fn foreground_는_우리_프로세스_창이면_무시한다() {
        let mut s = Session::default();
        // 앱(100)과 WebView2 브라우저 프로세스(200) 모두 무시
        for pid in [100, 200] {
            for shell in [false, true] {
                assert!(s
                    .on_message(HelperMessage::Foreground { pid, shell }, &ctx(true))
                    .is_empty());
            }
        }
    }

    #[test]
    fn foreground_는_셸_밖_앱이면_클릭_포커스를_허용하고_셸이면_막는다() {
        let mut s = Session::default();
        let fg = |pid, shell| HelperMessage::Foreground { pid, shell };
        // 다른 앱: 허용(열려 있으면 로그도)
        let got = s.on_message(fg(300, false), &ctx(true));
        assert_eq!(got[0], Action::SetActivatable(true));
        assert!(matches!(got.get(1), Some(Action::Log(_))));
        assert_eq!(got.len(), 2);
        // 닫혀 있으면 로그 없이 허용 상태만
        assert_eq!(
            s.on_message(fg(300, false), &ctx(false)),
            vec![Action::SetActivatable(true)]
        );
        // 셸(탐색기·바탕화면): 막는다
        assert_eq!(
            s.on_message(fg(300, true), &ctx(true)),
            vec![Action::SetActivatable(false)]
        );
    }

    #[test]
    fn 복사_키는_렌더러에_선택_텍스트를_묻는다() {
        let mut s = Session::default();
        assert_eq!(
            s.on_message(key(KeyName::Copy), &ctx(true)),
            vec![Action::RequestCopy]
        );
    }

    #[test]
    fn 헬퍼가_다시_뜨면_남아_있는_텍스트_선택을_다시_알린다() {
        let mut s = Session::default();
        let ready = || HelperMessage::Ready {
            version: "1".into(),
            pid: 5,
        };
        let none = s.on_message(ready(), &ctx(true));
        assert!(!none.contains(&Action::Send(HostMessage::TextSelection { selected: true })));
        let got = s.on_message(
            ready(),
            &Ctx {
                text_selected: true,
                ..ctx(true)
            },
        );
        assert_eq!(
            got.last(),
            Some(&Action::Send(HostMessage::TextSelection { selected: true }))
        );
    }

    /// 호스트가 헬퍼에 보낸 state 를 모은다. `sync` 는 controller.sync_helper_state 와 같은 규칙(상태를 읽어 그대로 보냄)이다.
    struct Wire(Vec<bool>);

    impl Wire {
        fn sync(&mut self, st: &PreviewState) {
            match state_message(st.is_open()) {
                HostMessage::State { preview_open } => self.0.push(preview_open),
                other => unreachable!("{other:?}"),
            }
        }

        /// "true 를 보냈으면 반드시 false 가 뒤따른다": 호스트가 닫힌 상태일 때 헬퍼가 마지막으로 받은 값은 false 여야 한다
        fn assert_settled(&self, st: &PreviewState) {
            assert!(!st.is_open());
            assert_eq!(self.0.last(), Some(&false), "{:?}", self.0);
        }
    }

    /// 세션이 열기 요청에 true 를 먼저 보낸다는 전제(위의 테스트)에서, 표시가 끝나지 않는 경로마다 false 가 뒤따른다.
    fn session_sends_true() -> Wire {
        let mut s = Session::default();
        s.on_message(key(KeyName::Space), &ctx(false));
        let got = s.on_message(selection(Some(1), &["a"], Some("a")), &ctx(false));
        assert_eq!(got[0], Action::Send(state_message(true)));
        Wire(vec![true])
    }

    #[test]
    fn state_true_를_보낸_뒤_표시가_끝나지_않으면_false_가_뒤따른다() {
        // 큐에서 기다리는 동안 닫혀 폐기: 닫기가 false 를 보내고, 폐기 경로도 한 번 더 상태를 알린다
        let st = PreviewState::default();
        let mut wire = session_sends_true();
        let epoch = st.epoch();
        st.hide();
        wire.sync(&st); // Controller::close
        assert!(st.begin_in("a", Some(epoch)).is_none());
        wire.sync(&st); // show_in 의 폐기 경로
        wire.assert_settled(&st);

        // 닫기 알림 뒤에 낡은 true 가 도착해도(스레드 순서가 엇갈림) 폐기 경로가 바로잡는다
        let st = PreviewState::default();
        let epoch = st.epoch();
        st.hide();
        let mut wire = Wire(vec![false, true]);
        assert!(st.begin_in("a", Some(epoch)).is_none());
        wire.sync(&st);
        wire.assert_settled(&st);

        // 열기를 포기(렌더러 준비 시간 초과·창 없음)
        let st = PreviewState::default();
        let mut wire = session_sends_true();
        let (seq, _) = st.begin("a");
        wire.sync(&st);
        assert_eq!(wire.0.last(), Some(&true));
        st.abort_open(seq);
        wire.sync(&st);
        wire.assert_settled(&st);

        // 창 표시 실패
        let st = PreviewState::default();
        let mut wire = session_sends_true();
        let (seq, _) = st.begin("a");
        wire.sync(&st);
        st.fail_show(seq);
        wire.sync(&st);
        wire.assert_settled(&st);

        // 닫기 도중에 새 열기가 시작돼 있으면 true 가 맞다(닫기 알림이 새 열기를 끄지 않는다)
        let st = PreviewState::default();
        let mut wire = session_sends_true();
        st.hide();
        st.begin("b");
        wire.sync(&st);
        assert_eq!(wire.0.last(), Some(&true));

        // 열려 있지 않아도 닫기는 false 를 보낸다(멱등)
        let st = PreviewState::default();
        let mut wire = Wire(vec![]);
        st.hide();
        wire.sync(&st);
        wire.assert_settled(&st);
    }

    #[test]
    fn 네이티브_응답과_모르는_메시지() {
        let mut s = Session::default();
        assert_eq!(
            s.on_message(
                HelperMessage::NativePreview {
                    id: 4,
                    status: NativeStatus::Shown,
                    message: None
                },
                &ctx(true)
            ),
            vec![Action::NativeResult {
                id: 4,
                status: NativeStatus::Shown,
                message: None
            }]
        );
        assert!(s.on_message(HelperMessage::Unknown, &ctx(true)).is_empty());
        assert!(s
            .on_message(HelperMessage::Pong { id: 1 }, &ctx(true))
            .is_empty());
    }
}
