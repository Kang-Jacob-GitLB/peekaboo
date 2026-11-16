# Electron → Tauri 2 전환 계획

> 상태: **완료**(M8 반영됨, 2026-11-16). 정식 릴리스는 Tauri 로 나가고 Electron 호스트 코드는 제거했다("M8 에서 구현된 것"). 이 문서는 전환 기록이며, 아래 본문의 `src/main/…`·Electron 코드 근거는 전환 당시(제거 전) 저장소 기준이다. M0 판정(2·3번 제외)을 마쳤고 사용자 결정으로 전환을 계속한다("M0 판정 결과" 절).
> M1~M4(헬퍼·네이티브 미리보기·창 동작·복사·WebView2 보호·트레이·설정·패키징)가 main 에 들어갔고, M5(업데이터·옛 Electron 설치본 정리 훅)가 구현됐다. M6 베타(v1.0.0-beta.1)를 게시했고, M7 에서 `release.yml` 을 Tauri 로 바꿨다.
> M5 의 설치기 훅·업데이트는 **Windows Sandbox 리허설을 아직 하지 않았다**(`docs/tauri-rehearsal.md`). 코드·NSIS 문법·업데이트 확인/내려받기/서명 검증(로컬 서버)까지만 확인했다.
> 이 문서를 처음 쓸 때(2026-11-13)는 공식 문서(v2.tauri.app, docs.rs)·wry·tao 소스·Microsoft Learn 만 근거였고, 이후 실측으로 바뀐 내용은 해당 절에 표시했다.
> 근거 표기 `파일:줄` 은 처음 쓴 시점(67710ca)의 저장소 기준이다.

## 0. 결론

**조건부로 진행한다.**

1. **M0**(머지하지 않는 draft PR): 포커스를 뺏지 않는 WebView2 창과 네이티브 미리보기 겹치기가 되는지 판정한다. 안 되면 보류한다.
2. **M1**(Electron 위): 렌더러를 런타임과 무관하게 만든다(호스트 어댑터, 파일 해석을 렌더러로 이동). Electron 에도 이득이라 손해가 적다.
3. **M2~M6**: Tauri 를 병행 구현한다. 이 기간 릴리스는 계속 Electron 으로 나간다.
4. **M7** 전환, **M8** Electron 코드 제거.

핵심 결정:

- **C# 헬퍼는 그대로 둔다.** 별도 프로세스와 JSON Lines IPC v1 을 유지하고 `config.hostPids` 필드 하나만 추가한다.
- **main 의 Node 코드는 둘로 나눈다.** 텍스트 디코딩·zip/tar 목록은 렌더러로(`peek` Range 요청 위에서), 파일 정보·폴더 목록·MotW 검사는 Rust 커맨드로.
- **Electron 이 그냥 주던 기능은 WebView2 COM(webview2-com)으로 직접 구현한다.** iframe 이동 차단, 우클릭 복사, 렌더러 크래시 감지, 캡처, 브라우저 단축키 끄기.
- **기존 Electron 사용자에게 자동 업데이트 브리지를 두지 않는다.** 새 설치본을 직접 받아 설치하고, Tauri 설치기가 옛 Electron 설치본을 제거한다(6절).

## 1. 결정 사항

| 항목 | 결정 |
|---|---|
| 진행 방식 | M0 시험부터. 아래 판정 항목이 대안 두 가지로 풀리지 않으면 보류 |
| 첫 Tauri 릴리스 | `feat!` → 1.0.0 |
| WebView2 런타임 | 설치기 포함(`embedBootstrapper`), 2026-11-16 사용자 결정 — 이유: 기본 NSIS 템플릿이 내려받은 부트스트래퍼를 서명 확인 없이 실행 |
| Electron → Tauri 업데이트 브리지 | **두지 않음.** 사용자가 직접 설치, 설치기가 옛 버전 제거 |
| 알림 클릭으로 업데이트 설치 | 포기. 알림은 안내만 하고 트레이 "다시 시작하여 업데이트"로 설치 |
| 업데이터 서명 키 | 비밀번호 없는 키 하나를 저장소 시크릿으로(승인 Environment 없음), 키 파일은 복사본 하나만 보관. 2026-11-16 사용자 결정 — 이유: 개인 프로젝트·개인 사용. 키 생성은 저장소 관리자가 직접 |
| 미정(해당 단계에서 결정) | `%APPDATA%\Peekaboo` 의 Chromium 캐시 잔재 삭제, "모든 사용자" 설치본 처리, 베타 채널, 헬퍼 Rust 재작성 여부 |

## 2. Electron 의존 지점

분류: **코어** = Tauri 코어, **플러그인** = 공식 플러그인, **직접** = Rust/windows-rs/webview2-com 직접 구현.

| 기능 | Electron (근거) | Tauri 2 대응 | 분류 | 위험 |
|---|---|---|---|---|
| 단일 인스턴스 | `requestSingleInstanceLock` (`src/main/index.ts:19`) | `tauri-plugin-single-instance`(맨 먼저 등록) | 플러그인 | 하 |
| 창이 없어도 상주 | `window-all-closed` (`index.ts:24`) | `RunEvent::ExitRequested` 에서 `prevent_exit` | 코어 | 하 |
| 브라우저 단축키 끄기 | `Menu.setApplicationMenu(null)` (`index.ts:32-34`) | `ICoreWebView2Settings3::SetAreBrowserAcceleratorKeysEnabled(false)` | 직접 | 중 |
| 미리보기 창 생성 | `BrowserWindow` (`src/main/preview-window.ts:290-311`) | `WebviewWindowBuilder`: `decorations(false)`, `always_on_top`, `focusable(false)`, `skip_taskbar`, `shadow(false)` 등 | 코어 | 하 |
| 포커스 없이 표시 | `showInactive()` (`preview-window.ts:129`) | tao 는 조건부로만 `SW_SHOWNOACTIVATE`. HWND 직접 `SetWindowPos(SWP_NOACTIVATE)` 대안 | 직접 | **상(판정 항목)** |
| 클릭 시 포커스 허용 | `setFocusable`+`setSkipTaskbar` (`preview-window.ts:486-489`) | `set_focusable`(tao: `WS_EX_NOACTIVATE` 토글), 이후 `set_skip_taskbar` 재적용 | 코어 | 중 |
| 새 창 차단 | `setWindowOpenHandler` (`:316`) | wry 기본값이 거부 | 코어 | 하 |
| 메인 프레임 이동 차단 | `will-navigate` (`:317`) | `on_navigation(\|_\| false)` | 코어 | 하 |
| iframe 이동 차단 | `will-frame-navigate` (`:319-321`) | `add_FrameNavigationStarting` + `put_Cancel(true)` | 직접 | 중 |
| 우클릭 복사 | `context-menu` `selectionText` (`:326`) | `ICoreWebView2_11::add_ContextMenuRequested` → `SelectionText` | 직접 | 중 |
| 렌더러 사망·무응답 | `render-process-gone`, `unresponsive` (`:327-333`) | `add_ProcessFailed` | 직접 | 중 |
| 헤더 드래그 이동 | `-webkit-app-region` (`src/renderer/styles/base.css:67,85`), `will-move` (`:342-344`) | 앱 커맨드 `start_drag` → `userMoved=true` 후 `start_dragging()` | 코어+앱 | 중 |
| 개발용 캡처 | `capturePage` (`:267-272`) | `ICoreWebView2::CapturePreview(PNG)` | 직접 | 하 |
| HWND 전달 | `getNativeWindowHandle` (`:437`) | `Window::hwnd()` | 코어 | 하(겹치기는 상) |
| 모니터·작업 영역 | `screen.*` (`:500-507`) | `cursor_position`, `monitor_from_point`, `work_area()`(물리 픽셀) | 코어 | 중(DIP 환산) |
| 렌더러 ↔ main IPC | `ipcMain`, `fromPreview` (`:414-480`) | `#[tauri::command]` + `emit_to`, 커맨드마다 라벨 검사, `AppManifest` 로 ACL | 코어 | 중 |
| preload API | `contextBridge` (`src/preload/index.ts:36`) | 렌더러 어댑터 `src/renderer/host/tauri.ts` | 앱 | 중 |
| 파일 서빙 | `protocol.handle` + `net.fetch` (`src/main/protocol.ts:9-29`) | `register_asynchronous_uri_scheme_protocol`, Range 직접 구현 | 코어+직접 | **상** |
| UNC 차단 | `webRequest.onBeforeRequest` (`protocol.ts:37-50`) | 출처가 http 로 바뀌고 CSP 에 `file:` 없음. 새면 `WebResourceRequested` 필터 | 직접 | 중(실측 필요) |
| 헬퍼 실행·감시 | `spawn` + `readline` (`src/main/helper-supervisor.ts:96-109`) | `tokio::process` + `CREATE_NO_WINDOW`, `bundle.externalBin` | 직접 | 중 |
| 트레이 | `Tray`, `Menu` (`src/main/tray.ts`) | `TrayIconBuilder`, 상태가 바뀌면 메뉴 재생성 | 코어 | 하 |
| 클립보드·폴더 열기 | `clipboard`, `shell.openPath` | clipboard-manager, opener 플러그인(Rust 에서만 호출) | 플러그인 | 하 |
| 자동 업데이트 | `electron-updater` (`src/main/updater.ts:47-124`) | `tauri-plugin-updater`(minisign 서명 필수, `latest.json`) | 플러그인 | 중 |
| 알림 클릭 설치 | `Notification` `click` (`updater.ts:84-88`) | 데스크톱 클릭 이벤트 없음 → 트레이로 대체 | — | 기능 손실 |
| 설정·로그 경로 | `getPath('userData')` (`settings.ts:21`, `logger.ts:11`) | `%APPDATA%\Peekaboo` 를 명시적으로 사용(경로 불변) | 직접 | 하 |
| 시작 시 실행 | `setLoginItemSettings` (`settings.ts:47-53`) | HKCU Run 값 직접 관리, 값 이름 `io.github.kang-jacob-gitlb.peekaboo` 유지 | 직접 | 하 |
| CSP | `<meta>` (`src/renderer/index.html:7`) | `tauri.conf.json` `app.security.csp`, `connect-src` 에 `ipc: http://ipc.localhost` | 코어 | 중 |
| 패키징 | `electron-builder.yml` | Tauri NSIS(`currentUser`), `externalBin`, `installerHooks` | 코어 | 중 |

## 3. 위험이 큰 항목의 해법

### 3.1 네이티브 미리보기 핸들러 겹치기

헬퍼는 넘겨받은 HWND 아래에 `WS_CHILD|WS_CLIPCHILDREN|WS_CLIPSIBLINGS` 컨테이너를 만든다(`helper/Shell/NativePreviewHost.cs:304-306`).
Tauri(wry)는 최상위 창 아래 `WRY_WEBVIEW` 자식 창(`WS_CLIPSIBLINGS` 없음)에 WebView2 를 둔다. 그래서 헬퍼 컨테이너는 그 형제가 되고,
웹뷰가 다시 그릴 때 컨테이너를 덮거나 깜박일 수 있다.

- **A안(기본)**: 구조 유지. 헬퍼가 open/resize 때마다 `SetWindowPos(container, HWND_TOP, …, SWP_NOACTIVATE)`.
- **B안**: 컨테이너를 소유 팝업 창(`WS_POPUP`, owner = 미리보기 HWND, `WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW`)으로. 창 이동마다 resize 를 보낸다. IPC 필드는 그대로.
- `shadow(false)` 여야 클라이언트 원점 = 뷰포트 원점 전제가 유지된다. DPI 는 양쪽 모두 Per-Monitor V2.

### 3.2 `peek` 프로토콜

- URL 이 `peek://file/…` 에서 `http://peek.localhost/<encodeURIComponent(path)>` 로 바뀐다. `toPeekUrl`/`fromPeekUrl`(`src/shared/channels.ts:25-37`)은 어댑터로 옮긴다.
- wry 는 응답 본문을 통째로 메모리에 넘긴다(스트리밍 불가). Range 는 단일 구간(`a-b`, `a-`, `-n`)만 지원하고 응답을 최대 4MB 로 잘라 206 으로 준다. 맞지 않으면 416.
- Range 없는 전체 요청은 128MB 를 넘으면 413(렌더러는 이미 100MB 로 제한, `src/renderer/plugins/read-binary.ts:4,8`).
- `isAllowedPath` 는 의미 그대로 옮긴다: 현재 **표시 중인 창의 라벨** 하나와 경로가 바이트 단위로 정확히 일치할 때만 허용. 프로토콜과 모든 커맨드가 같은 함수를 쓴다.
- 필수 테스트: 한글, `#`·`%`·`+`·공백, UNC, `\\?\` 긴 경로, NFC/NFD 이름의 WebView2 왕복.

### 3.3 보안 경계

| Electron 의 경계 | Tauri 대응 |
|---|---|
| `sandbox`·`contextIsolation` (`preview-window.ts:304-310`) | WebView2 렌더러는 항상 Chromium 샌드박스. 단 `__TAURI_INTERNALS__` 가 메인 월드에 있어 페이지 스크립트가 IPC 를 부를 수 있다. 신뢰하지 않는 내용이 메인 프레임에서 스크립트를 실행할 수 없다는 전제(스크립트 없는 iframe, DOMPurify)가 유지되는 한 문제없다 |
| 송신자 검증 `fromPreview` (`:415`) | 앱 커맨드는 기본이 "모든 창 허용"이므로 `build.rs` `AppManifest::commands` 로 ACL 아래에 두고 capability 는 `preview-*` 창에만. 커맨드마다 라벨과 경로 검사 |
| 코어 권한 | `core:event:allow-listen/unlisten` 만. `core:window:*` 은 주지 않는다 |
| sandbox iframe (`src/renderer/plugins/html/index.ts:24`) | **주입됨(M0 실측)**: sandbox iframe 안에도 `__TAURI_INTERNALS__` 가 들어 있다. `allow-scripts` 가 없어 스크립트가 실행되지 않으므로 호출할 수단이 없을 뿐이다. 그래서 `allow-scripts` 를 추가하면 즉시 IPC 가 노출된다 — 불변식으로 지킨다(아래 "M0 판정 결과" 6) |
| `will-frame-navigate`, 새 창 차단 | `FrameNavigationStarting` 취소, wry 기본 거부 |
| UNC 차단 | **실측 전에는 막힌다고 단정하지 않는다.** HTML·Markdown 의 `<img src="file://호스트/…">` 로 SMB 연결이 생기는지 확인하고, 새면 `WebResourceRequested` 로 403 |
| 클립보드 | 우클릭은 호스트 이벤트(`ContextMenuRequested` 의 선택 텍스트를 호스트가 클립보드에 쓴다). Ctrl+C 는 기존 `pendingCopy` id 가드를 Rust 로 옮겼다: 헬퍼의 `key{copy}` 로 호스트가 요청 id 를 발급하면 렌더러가 같은 id 로 `copy_text` 커맨드를 불러 문자열을 넘기고, 호스트는 id 가 지금 기다리는 1회용 번호와 같을 때만 그 문자열을 클립보드에 쓴다(요청 없이 온 응답·낡은 id·두 번째 응답은 버린다). 즉 **렌더러 문자열을 받는 커맨드가 하나 있고**, 그 안전은 (1) 헬퍼 키로만 id 가 발급된다는 점과 (2) 신뢰하지 않는 내용이 메인 프레임에서 스크립트를 실행할 수 없다는 전제(스크립트 없는 sandbox iframe, DOMPurify)에 기댄다. 추가 방어: 16MiB(UTF-8 바이트) 초과 거부, 복사 알림에 글자·줄 수 표시(보이지 않는 글자를 복사한 것을 알아채게) |

isolation pattern 은 쓰지 않는다. 이 앱의 위협은 프런트엔드 의존성이 아니라 미리볼 파일이고, 그 내용은 스크립트를 실행할 수 없는 구조다.

### 3.4 미리보기 창 동작

창 상태 기계(`PreviewWindow`)를 Rust 로 1:1 이식하되, 포커스 관련 동작은 HWND 직접 제어 경로를 준비한다.

- **두 번째 이후 show**: tao `show()` 는 `MARKER_DONT_FOCUS` 가 있을 때만 `SW_SHOWNOACTIVATE`. 같은 창을 hide→show 반복하므로(`preview-window.ts:162-164,277`) 핵심 시험 항목이다. 대안은 `SetWindowPos(HWND_TOPMOST, …, SWP_NOACTIVATE|SWP_SHOWWINDOW)` / `ShowWindow(SW_HIDE)` 직접 호출.
- **클릭**: WebView2 콘텐츠 창은 다른 프로세스(msedgewebview2)의 자식이라, 비활성 창 클릭 시 활성화 여부를 실측해야 한다.
- **`<select>` 드롭다운**: 팝업이 msedgewebview2 소유 최상위 창이라 헬퍼가 "다른 앱 전환"으로 오인할 수 있다. `config.hostPids`(앱 PID + `BrowserProcessId`)를 추가 필드로 보낸다. 바뀌는 곳: `helper/HelperCore.cs`, `helper/Input/FocusInspector.cs`, `helper/Ipc/Messages.cs`, `src/shared/ipc-types.ts`, `docs/ipc.md`.
- **헤더 드래그**: WebView2 `IsNonClientRegionSupportEnabled` 는 시스템 메뉴·더블클릭 최대화가 함께 켜지고 사용자 이동을 구분할 수 없어 쓰지 않는다.
- **spare 창**: M3b 에서 **이식하지 않기로 했다.** 창 하나를 계속 쓰고, 렌더러가 죽으면 `ProcessFailed` 에서 페이지를 다시 불러온다(`src-tauri/src/guard.rs`). WebView2 가 렌더러 프로세스를 공유하면 교체 효과가 없을 수 있고, 창을 만드는 호출은 Windows 에서 동기 컨텍스트 교착 위험이 있어서다.
- **크기 계산**: Tauri 는 물리 픽셀. 작업 영역 85%, 최소 420×240 등 상수는 DIP 기준을 유지하고 `scale_factor` 로 환산.

### 3.5 헬퍼

- 1차 범위에서 C# 헬퍼를 그대로 쓴다. `helper-supervisor.ts` 의 재시작 백오프 `[1,2,4,8,15,30]s`, 60초 안정 시 초기화, 2초 ping·3회 무응답 kill, stderr RAW 로그를 Rust 로 옮긴다.
- 부모가 죽으면 stdin EOF 로 헬퍼가 스스로 끝나는 보장(`helper/Program.cs:74,103`)은 그대로 유지된다.
- shell 플러그인 sidecar 는 쓰지 않는다(JS 권한 표면 증가, 줄 단위 파싱이 오히려 우회적).
- **헬퍼를 Rust 메인 프로세스로 흡수하지 않는다.** 저수준 키보드 훅이 UI·웹뷰 프로세스에 있으면 `LowLevelHooksTimeout` 초과 시 Windows 가 훅을 조용히 해제해 Space 가 먹통이 되고, `DoPreview` 의 수 초짜리 STA 블록이 앱 전체에 영향을 준다.

### 3.6 main 의 Node 코드

| 함수 | 근거 | 옮길 곳 |
|---|---|---|
| `getFileInfo` | `src/main/file-service.ts:7-25` | Rust |
| `listDirectory` | `file-service.ts:62-76` | Rust 커맨드 `list_dir` |
| `isFromInternet` | `preview-window.ts:530-538` | Rust(`:Zone.Identifier` ADS) |
| `readTextHead` + `decode` | `file-service.ts:28-60` | 렌더러(`Range: bytes=0-max`, `TextDecoder('euc-kr')`) |
| `listArchive`(zip) | `file-service.ts:95-251` | 렌더러(`Buffer` → `DataView`, crc32 JS 구현) |
| `listTar`/gzip | `src/main/tar-list.ts:22-302` | 렌더러(`ByteSource` 추상화 재사용, `DecompressionStream('gzip')`) |

렌더러로 옮기면 신뢰하지 않는 바이트 해석이 특권 main 에서 샌드박스로 내려가고, 보안 규칙(Unicode Path CRC 검증, pax 숫자 엄격 파싱 등)을 Rust 로 다시 쓰지 않아도 된다.

## 4. 단계

원칙: 모든 PR 은 `main` 을 릴리스 가능한 상태로 둔다. M6 까지 릴리스는 Electron 으로 나가고, 공존 기간 PR 제목은 릴리스가 나가지 않는 타입(`refactor`·`build`·`ci`·`docs`·`test`)을 쓴다.

| 단계 | 내용 | 검증 |
|---|---|---|
| **M0** 시험 구현(draft, 머지 안 함) | 최소 Tauri 앱: 미리보기 창 1개, `peek` 프로토콜, 기존 렌더러를 임시 어댑터로, C# 헬퍼 연결. Rust(MSVC) 툴체인 선행 | 아래 판정 항목 + 크기·메모리 실측. 결과는 이 문서에 추가 |
| **M1** 렌더러 런타임 분리(Electron 위) | `src/renderer/host/{index,electron}.ts`, 파일 해석을 `host/file-ops/` 로 이동하고 main 의 `readText`/`listArchive` IPC 제거, CSP 대상별 주입, `config.hostPids` 추가. `PeekabooApi` 는 그대로 | 기존 Node 구현과 새 렌더러 구현의 출력 JSON 비교(`node --test`), 포맷별 캡처 전후 비교 |
| **M2** Tauri 골격 | `src-tauri/`, `vite.tauri.config.ts`(pdfjs 자산 플러그인 공용화), `host/tauri.ts`, 창 생성·표시, `peek` Range, `list_dir`/`file_info`, 디버그 캡처 | `cargo test`(Range 파서, URL 왕복, isAllowed), Electron/Tauri 캡처 나란히 비교, 영상 seek·100MB PDF·UNC 실측 |
| **M3** 헬퍼·창 동작 | supervisor·native bridge, `PreviewWindow` 상태 기계, 복사 2종, FrameNavigationStarting, ProcessFailed, 가속키 끄기 | GUI 체크리스트(사용자가 PC 를 쓰지 않을 때만), `shown in`/`rendered` 지연을 Electron 과 비교 |
| **M4** 트레이·설정·패키징 | 트레이, 설정, Run 키, 로그, single-instance, `externalBin`, NSIS `currentUser`, 별도 수동 워크플로 `tauri-build.yml`(릴리스 파이프라인과 분리, 발행 안 함) | 깨끗한 계정에 아티팩트 설치 |
| **M5** 업데이터·옛 버전 제거 | `tauri-plugin-updater`, `src-tauri/windows/hooks.nsh`(6절), `latest.json` 생성(`scripts/update-feeds.mjs`), 서명 키 시크릿 | 옛 Electron 버전 3개(0.2.0, 중간, 최신) 위에 Tauri 설치본 설치 리허설, Tauri → Tauri 업데이트는 로컬 서버로(`docs/tauri-rehearsal.md`, Windows Sandbox 에서만) |
| **M6** 베타 | 프리릴리스 태그를 수동 발행(`gh workflow run tauri-build.yml -f prerelease_tag=v1.0.0-beta.1`: 빌드·서명·피드 검증 뒤 프리릴리스로 게시), 테스터가 직접 설치(Electron 사용자는 `allowPrerelease=false` 라 받지 않음, `updater.ts:54`) | 1~2주 실사용: 포커스 회귀, Office 핸들러, 메모리 |
| **M7** 전환(반영됨) | `feat!: Tauri 2 로 전환`. `release.yml` 기본 빌드를 Tauri 로, README·CLAUDE.md 갱신 | 사람이 지켜보는 시간에 `gh workflow run nightly-release.yml` |
| **M8** 정리(반영됨) | `src/main`, `src/preload`, `electron-builder.yml`, `electron.vite.config.ts`, electron 의존성 제거 | typecheck, build, cargo test |

### M0 판정 항목

하나라도 실패하고 대안 두 가지(tao API → HWND 직접 제어 → B안)로도 풀리지 않으면 **전환을 보류한다.**

1. 탐색기가 포그라운드일 때 Space → Esc → Space 로 **두 번째 표시**까지 탐색기 포커스가 유지되고 방향키로 다음 파일로 넘어간다.
2. 본문 클릭, 드래그 선택, 휠 스크롤, ✕, 헤더 드래그 뒤에도 탐색기 포커스가 유지된다.
3. raw 플러그인 `<select>` 를 연 뒤에도 포커스가 유지되고 헬퍼가 다른 앱 전환으로 오인하지 않는다.
4. Office 핸들러 창이 깜박임 없이 WebView2 위에 겹치고 리사이즈를 따라간다.
5. `peek` 로 영상을 재생·seek 할 수 있고 100MB PDF 가 열린다.
6. sandbox iframe 안에서 IPC 객체를 쓸 수 없고 스크립트가 실행되지 않는다. `file://호스트/` 요청이 나가지 않는다.
7. 실측: 설치본·설치 폴더 크기, 유휴 메모리(프로세스별), 콜드 스타트, Space→표시 지연.

### M0 판정 결과

**결론: 전환을 계속한다(2026-11-13, 사용자 결정).** 판정 1·4·5·6 은 통과(4·6 은 조건부), 2·3 은 사용자 입력이 필요해 아직 실측하지 못했다.
메모리 절감이 보류 기준 2(50MB 미만)에 해당했으나, 설치·업데이트 크기 이득을 근거로 계속하기로 했다.

1. **포커스 유지(프로그램 표시): 통과.** tao·HWND 직접 두 방식 모두 5/5, 숨긴 채 만든 창의 첫 표시 포함. 근거 PR #22.
2. **클릭·드래그 뒤 포커스, 3. `<select>` 뒤 포커스: 미실측.** 사용자 입력이 필요하다. 사용자가 PC 를 비울 때 진행한다.
4. **네이티브 겹치기: 조건부 통과.** 헬퍼 컨테이너가 `WRY_WEBVIEW` 아래에 생성돼 그대로는 0/25, 컨테이너를 `HWND_TOP` 으로 올리면 5/5.
   `nativePreview.open.raise` 로 Tauri 에만 적용했다(#25). M3b 에서 PowerToys 핸들러로 화면 밖 재확인한다.
5. **미디어·대용량 PDF: 통과.** mp4(moov 앞/뒤)·webm 4종 재생·탐색 성공, 4MiB 이어받기 정상. 99MiB PDF 는 열렸으나 호스트 순간 메모리가 약 1,000MiB 였다.
   M3b 에서 4MiB 조각 읽기(`read-binary.ts`)로 바꿔 피크 약 67MiB. 실측 중 Electron 의 기존 버그(비표준 스킴 + `crossorigin` 없음으로 두 번째 Range 응답 거부)를 발견해 #26 으로 고치고 v0.10.1 로 배포했다.
6. **iframe·UNC: 조건부 통과.** 스크립트 실행 0, UNC 연결 시도 0(대조군에서는 879건 검출).
   단 sandbox iframe 안에도 `__TAURI_INTERNALS__` 가 주입된다(계획서 예상과 다름). 스크립트가 꺼져 있어 호출 수단이 없을 뿐이며,
   **`allow-scripts` 를 추가하면 즉시 IPC 가 노출된다. HTML 미리보기 iframe 에 `allow-scripts` 를 넣지 않는 것을 불변식으로 한다.**
7. **크기·메모리·시작: 측정함.** 설치본 122.2MiB → 추정 약 8.6MiB(exe+헬퍼 LZMA), 설치 폴더 378MiB → 약 23MiB,
   유휴 Private 274.3 → 252.8MiB(절감 21.5MiB: WebView2 의 browser·crashpad·storage 프로세스가 Node main 몫을 대부분 채운다). 시작 시간은 차이를 판별하지 못했다.

**결정(2026-11-13, 사용자)**: 보류 기준 2(메모리 절감 50MB 미만)에 해당하나 설치·업데이트 크기 이득을 근거로 전환을 계속한다.

### 캡처 방식

`npm run build` → `cargo build` → `PEEKABOO_PREVIEW=<파일> PEEKABOO_CAPTURE=<png> src-tauri/target/debug/peekaboo.exe`.
이 모드에서는 single-instance 를 등록하지 않고, WebView2 데이터 폴더를 임시 폴더로 바꾸고, 헬퍼는 `PEEKABOO_HELPER=1` 일 때만 띄운다(키보드 훅 중복 방지).
CapturePreview 는 웹 콘텐츠만 찍는다(Office 네이티브 창 제외, `capturePage` 와 같음).

**화면 밖 모드**: `PEEKABOO_OFFSCREEN=1` 을 함께 주면(Electron·Tauri 공통, 디버그 전용) 창을 모든 모니터의 왼쪽 바깥에 두고 크기는 주 모니터 기준으로 계산한다.
위치는 한 곳(`place`)에서만 정해 처음 표시·fit 이 창을 화면 안으로 끌어오지 않고, 띄우기 직전 실제 창 사각형이 모니터와 겹치면 띄우지 않고 종료한다.
Chromium 이 가려진 창의 렌더를 멈추지 않게 Electron 은 `backgroundThrottling:false` + `disable-features=CalculateNativeWinOcclusion`,
Tauri 는 WebView2 브라우저 인자를 쓴다. Tauri 의 `additional_browser_args` 는 wry 의 기본 인자를 **대체**하므로 기본값(`--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`,
`--autoplay-policy=no-user-gesture-required`)을 합쳐 넣는다(`src-tauri/src/preview.rs` `OFFSCREEN_BROWSER_ARGS`). 이 모드에서는 헬퍼(Electron)·트레이도 띄우지 않고 캡처 뒤 종료한다.

## 5. CI·릴리스

- **`build.yml`**: 필수 체크 job 이름 `Typecheck · Build` 를 유지하고 같은 job 에 Rust 툴체인·캐시(SHA 고정), `npm run build:web:tauri`, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --locked` 를 추가한다. job 을 나누면 룰셋의 required check 를 바꿔야 한다. PR 시간은 콜드 캐시 +8~15분, 웜 캐시 +2~5분으로 추정.
- **`release.yml`**: `npm pkg set version` 유지, `tauri.conf.json` 의 `version` 이 `../package.json` 을 가리키게 하고 `Cargo.toml` 은 `0.0.0` 고정. 헬퍼 산출물을 `src-tauri/binaries/peekaboo-helper-x86_64-pc-windows-msvc.exe` 로 복사, `npx tauri build --bundles nsis`, 서명 키는 시크릿 `TAURI_SIGNING_PRIVATE_KEY`(+`_PASSWORD`). 산출물 이름 `Peekaboo-Setup-${version}-x64.exe` 유지. 메타데이터 확인 단계는 `latest.json`(버전, 서명, URL) 검사로 바꾼다.
- **`nightly-release.yml`**: 재사용 호출에 `secrets: inherit` 만 더해 서명 키를 `release.yml` 로 넘긴다.
- **M5 에서 구현된 것**: `tauri-build.yml`(수동)은 **빌드와 서명을 두 잡으로 나눈다.** 빌드 잡(`installer`)은 서명 개인키를 받지 않고(`CI` 환경에서 `scripts/tauri-build.mjs` 는 서명 산출물을 만들지 않고 기본 엔드포인트 외를 거부한다), 저장소의 공개키 파일 `src-tauri/updater.pub` 이 있을 때만 업데이터 설정을 얹어 설치본을 아티팩트로 올린다. 서명은 `sign` 잡(`needs: installer`)만 한다: 저장소 시크릿 `TAURI_SIGNING_PRIVATE_KEY`(+`_PASSWORD`)를 환경 변수로 받아 `npx --yes @tauri-apps/cli@2.12.4 signer sign --app-version <버전> <설치본>` 한 번으로 서명하고, `scripts/update-feeds.mjs` 가 서명을 공개키·설치본·버전과 대조해 검증한 뒤 `latest.json` 을 만든다. `updater.pub` 가 없으면 `sign` 잡은 건너뛰고, 시크릿이 없으면(포크 등) 서명 단계만 건너뛴다. 발행은 하지 않는다. `release.yml` 변경은 M7.
- **M7 에서 구현된 것**: `release.yml` 이 `build`(Tauri 설치본, 서명 시크릿 없음) → `sign`(서명·`update-feeds.mjs` 검증·`latest.json`) → `release`(초안 → 공개) 3잡이 됐다. 버전 결정·트리거 3종·버전 정규식·태그 생성은 그대로다. 빌드 잡은 `npm run dist:tauri` 와 같은 단계를 개별로 돌려 헬퍼에 `-p:Version`·`-p:InformationalVersion` 을 넘긴다(옛 Tauri 시험 빌드는 헬퍼 버전이 0.0.0 으로 찍혔다). `sign` 잡은 발행하는 실행(태그 push, `workflow_call`)에서만 돌고, 서명 시크릿이나 `src-tauri/updater.pub` 가 없으면 **실패**해 서명 없는 정식 릴리스가 나가지 않는다(수동 점검 실행은 서명·발행하지 않고 설치본만 아티팩트로 올린다). `latest.json` 의 노트는 재사용 호출이면 입력 노트, 태그 push 면 태그 메시지, 없으면 기본값이다. 릴리스에는 설치본·`.sig`·`latest.json` 을 올리고 `latest.yml`·blockmap 은 더 이상 만들지 않는다. `tauri-build.yml` 은 시험·베타 프리릴리스 게시용으로 남겨 둔다. `scripts/next-version.mjs` 의 릴리스 노트 꼬리는 1.0.0 이상의 자동 업데이트와 0.x 사용자의 직접 설치 안내로 바꿨다.
- **M8 에서 구현된 것**: Electron 호스트를 걷어냈다. 지운 것: `src/main/`, `src/preload/`, `electron.vite.config.ts`, `electron-builder.yml`, `host/electron.ts` 와 `createHost()` 의 Electron 분기(`VITE_HOST` 컴파일 타임 상수·`window.peekaboo` 타입 포함), Electron IPC 채널과 헬퍼 메시지 TS 타입(`src/shared/channels.ts`·`ipc-types.ts`; 헬퍼 프로토콜의 정의는 `docs/ipc.md`·`helper/Ipc/Messages.cs`·`src-tauri/src/helper.rs`), `index.html` 의 CSP `<meta>`(Tauri 는 `tauri.conf.json` 의 CSP 를 쓴다)와 그것을 빼던 vite 플러그인, `electron`·`electron-builder`·`electron-vite`·`electron-updater` 의존성. `test/offscreen.test.ts` 와 `file-ops.test.ts` 의 Range 헤더·peek 응답 조립 시험은 대응하는 Rust 시험(`layout.rs` 의 offscreen, `peek.rs` 의 parse_range·build_response)이 있어 지웠다(fake fetch 용 Range 해석만 `test/helpers/range.ts` 로 옮김). 이름이 바뀐 것: `vite.tauri.config.ts` → `vite.config.ts`, `npm run build:web:tauri` → `npm run build`(렌더러, 산출물 `out/renderer-tauri` → `out/renderer`), `npm run dist:tauri` → `npm run dist`, `tsconfig.node.json` 은 vite 설정·시험만 본다. `build.yml` 의 job 이름 `Typecheck · Build` 는 그대로다. 되돌릴 필요가 생기면 8절 1번의 Electron 코드는 M7 직후 커밋(`224b034`)에 있다.
- **서명**: 업데이터 minisign 키는 필수이고 끌 수 없다. **키를 잃으면 기존 Tauri 설치본은 다시는 자동 업데이트를 받지 못한다.** 키 파일 복사본을 하나 보관한다(잃으면 새 키로 바꾼 버전을 한 번 수동 설치). Authenticode 는 지금처럼 없다.
- **로컬 개발**: README "필요한 도구"에 Rust(MSVC) 추가.

## 6. 기존 Electron 사용자 이전

**자동 업데이트 브리지를 두지 않는다.** Tauri 릴리스는 `latest.yml` 을 발행하지 않으므로, Electron 설치본은 마지막 Electron 버전에 머물고 업데이트 확인은 실패로 끝난다. 사용자는 새 설치본을 직접 받아 설치한다.

Tauri NSIS 는 설치 경로(`%LOCALAPPDATA%\Peekaboo`)와 제거 키(`HKCU\…\Uninstall\Peekaboo`)가 Electron(`%LOCALAPPDATA%\Programs\Peekaboo`, 제거 키 GUID `a32366e0-3e8a-5400-a683-97d78e557198`)과 달라서,
그냥 설치하면 두 앱이 함께 떠 키보드 훅이 2개가 되고 Space 가 두 번 처리된다. 그래서 설치기 훅(`src-tauri/windows/hooks.nsh`)이 옛 버전을 정리한다.

1. **PREINSTALL**: HKCU 옛 제거 키에서 `UninstallString` 을 읽어 옛 제거 프로그램을 실행하고 기다린다(시작 시 실행 값은 아래 2번에서 옮긴다). 옛 제거 프로그램이 같은 이름의 바로가기를 지우므로 새 바로가기를 만들기 **전에** 한다(템플릿에서 `NSIS_HOOK_PREINSTALL` 은 `Section Install` 맨 앞, `CheckIfAppIsRunning`·바로가기 생성보다 앞이다).
   - **구현(M5)**: 제거 프로그램을 `$PLUGINSDIR` 로 복사해 `"<복사본>" /S /KEEP_APP_DATA /currentuser --updated _?=<설치 폴더>` 로 실행한다. electron-builder 26.18.0 이 자기 옛 버전을 지울 때 쓰는 방식(`node_modules/app-builder-lib/templates/nsis/include/installUtil.nsh` 의 `uninstallOldVersion`)을 그대로 따랐다.
     `_?=` 가 없으면 NSIS 제거 프로그램이 임시 폴더로 자신을 복사해 다시 실행하고 즉시 끝나 기다릴 수 없고, 복사본이 아니라 설치 폴더의 파일을 그대로 실행하면 제거 프로그램이 자기가 든 폴더를 지우지 못한다.
     `--updated` 는 사용자 데이터(`%APPDATA%\Peekaboo`) 삭제를 막는다(`uninstaller.nsh`: `--delete-app-data` 가 없고 `isUpdated` 면 `isDeleteAppData` 가 0). 실행 중인 옛 `Peekaboo.exe` 는 제거 프로그램이 직접 끝낸다.
     성공은 종료 코드 0 에 더해 HKCU 의 옛 제거 키가 사라졌는지 다시 읽어 확인한다. `ExecWait` 자체가 오류(복사본을 실행하지 못함)면 설치 폴더의 제거 프로그램을 제자리에서 한 번 실행한다(electron-builder 의 `TryInPlace`). 끝난 뒤에는 `_?=` 로 남은 제거 프로그램 파일과 비어 있는 옛 폴더를 정리한다.
     실패하면 1초 간격으로 3번까지 다시 시도하고, 그래도 안 되면 설치 로그에 남기고 **설치는 계속한다**(설치 자체를 막지 않는다 — 이 경우 두 앱이 함께 뜰 수 있어 수동 제거가 필요하다).
   - **안전 장치**: HKCU 의 `UninstallString` 은 사용자 권한으로 쓸 수 있는 값이라 신뢰하지 않는다. 설치기가 **승격**돼 있으면(토큰의 `TokenElevation`; `UserInfo::GetAccountType` 은 계정 종류를 묻는 것이라 승격 여부와 같다고 확신할 수 없어 쓰지 않았다. 확인 실패는 승격으로 간주. 비승격 실행에서 0 이 나오는 것은 임시 NSIS 설치기로 확인했고 승격 실행은 확인하지 못했다) 옛 제거를 건너뛴다. 경로는 `X:\` 로 시작하는 드라이브 절대 경로(UNC·`//`·장치 경로 거부)이고 파일 이름이 `Uninstall Peekaboo.exe` 여야 실행한다. HKLM 을 읽은 뒤 레지스트리 뷰는 템플릿 `.onInit` 의 `SetContext` 가 x64 에서 정한 64 그대로 둔다.
   - `$UpdateMode = 1`(앱 안 업데이트)에서는 이 단계를 건너뛴다. 옛 Electron 키가 없으면 아무것도 하지 않는다.
   - 훅 파일은 템플릿의 `Var UpdateMode` 보다 먼저 포함돼(`installer.nsi:32` vs `:70`) 함수 안에서 `$UpdateMode` 를 못 쓰므로, 검사는 매크로에서 하고 제거 로직만 함수(`PeekabooRemoveOldElectron`)로 뺐다.
2. **시작 시 실행 값 이동(PREINSTALL 안, 옛 제거 뒤)과 POSTINSTALL**: v0.1.0 등 옛 버전은 Run 값 이름이 달랐을 수 있어 이름으로 찾지 않는다. HKCU Run 의 모든 값 중 데이터가 옛 설치 폴더(`InstallLocation`, 없으면 제거 프로그램의 폴더)의 `Peekaboo.exe` 를 가리키는 것(따옴표·뒤따르는 인자 허용)을 찾아 기록한다. 옛 제거가 성공했으면 그 값을 지우고, 작업 관리자의 "사용 안 함" 상태(`StartupApproved\Run` 의 같은 이름 값, REG_BINARY)를 새 이름(`io.github.kang-jacob-gitlb.peekaboo`)으로 복사한다(값 이름이 바뀌면 상태가 이어지지 않기 때문. NSIS 내장 명령은 REG_BINARY 를 못 읽어 `advapi32` 를 `System::Call` 로 부른다). POSTINSTALL 은 옛 값이 있었을 때만 새 이름으로 `"$INSTDIR\Peekaboo.exe"` 를 쓴다(이미 있으면 덮는다). 원래 없었으면 새로 만들지 않는다.
3. "모든 사용자"로 설치한 옛 버전(HKLM)은 관리자 권한 없이 제거할 수 없어 건드리지 않는다. **결정(M5)**: 설치 로그(`DetailPrint`)에 남기고, 앱이 시작할 때(릴리스 빌드) HKLM·HKCU 의 옛 제거 키를 읽기만 해 로그에 수동 제거 안내를 남기며, HKLM 설치본이면 **처음 한 번** "이전 버전이 모든 사용자용으로 남아 있습니다 — 제어판(설정 > 앱)에서 제거해 주세요" 알림도 보낸다(`src-tauri/src/legacy.rs`. 이미 알렸는지는 설정 파일에 필드를 더하지 않고 데이터 폴더의 표식 파일 `legacy-all-users-notified` 로 둔다). 설치기는 32비트라 HKLM 은 `SetRegView 64` 로 읽는다.
4. **재설치("먼저 제거")**: 템플릿의 재설치 페이지는 설치 폴더의 `uninstall.exe` 를 `_?=<설치 폴더>` 로 직접 실행한다(`/UPDATE`·`/P` 는 해당 모드에서만 붙고 구별되는 인자가 없다). 이 제거 프로그램의 POSTUNINSTALL 이 Run 값을 지우면 이어지는 설치 뒤 시작 시 실행이 사라진다. `_?=` 는 `$CMDLINE` 에서 빠지므로 인자로는 못 가르고, `$EXEDIR` 로 가른다: 직접 실행이면 `$EXEDIR = $INSTDIR`, 설정 > 앱 등에서 실행하면 NSIS 가 임시 폴더(`~nsu*.tmp`)로 복사해 실행하므로 다르다(임시 NSIS 설치기로 실측). 같으면(재설치) 값을 지우지 않는다. 사용자가 "먼저 제거" 뒤 설치를 취소하면 값이 남지만 다음 설치가 다시 쓴다. 재설치 페이지를 떠나기 전에 부를 훅은 템플릿에 없다.
5. 실행 중 앱 검사: 새 설치기의 `CheckIfAppIsRunning` 은 PREINSTALL **뒤**에 돌고 옛 앱은 그 전에 제거 프로그램이 끝내므로 새 설치기가 옛 `Peekaboo.exe` 를 잡을 일은 없다고 본다. 다만 옛 제거 프로그램의 종료 처리(`allowOnlyOneInstallerInstance.nsh`)가 조용한 모드에서 멈추는지는 리허설로 확인한다(`sandbox-migrate.ps1` 이 옛 앱을 실행한 채로 설치한다).

설정과 로그는 `%APPDATA%\Peekaboo\{settings.json, logs\peekaboo.log}` 를 그대로 쓴다. WebView2 데이터는 Tauri 기본 위치에 둔다.

**Tauri 업데이터**(M5 구현, `src-tauri/src/updater.rs`): 엔드포인트 `https://github.com/Kang-Jacob-GitLB/peekaboo/releases/latest/download/latest.json`(프리릴리스 제외). 10초 뒤 한 번, 이후 6시간마다 확인하고 내려받아 메모리에 두었다가 트레이 "다시 시작하여 업데이트"(Electron 과 같은 문구·상태 표시) 또는 "종료"에서 설치한다. 알림은 안내만 한다("새 버전 X 를 받았습니다. 트레이 메뉴에서 다시 시작하여 업데이트"). OS 종료·로그오프 때 조용한 설치는 보장하지 못한다(지금의 `autoInstallOnAppQuit` 보다 약함).
- **설치 모드는 passive**: 플러그인이 설치본을 `/P /UPDATE /R`(`/P` passive, `/UPDATE` 업데이트 모드, `/R` 설치 뒤 앱 재실행)으로 실행한다(`tauri-plugin-updater 2.14.1 src/config.rs:41-58`, `updater.rs updater_parameters`). `/R` 는 passive 와 quiet(`/S`) 모두에 붙고(`restart_after_install` 기본값 true), 설치기의 `.onInstSuccess` 가 `$PassiveMode = 1` 또는 Silent 일 때만 `/R` 을 보고 `RunAsUser "$INSTDIR\Peekaboo.exe"` 로 다시 띄운다(생성된 `installer.nsi:762-771`).
  Electron 은 완전 조용한 설치(`isSilent`)였지만, 사용자가 메뉴를 눌러 시작하는 설치라 진행 표시줄이 보이는 편이 낫고 실패 대화상자도 보인다고 보아 passive 를 골랐다. quiet 로 바꾸려면 `scripts/updater-config.mjs` 의 `installMode` 한 줄이다.
- 확인·내려받기 요청 하나에 10분 제한을 둔다. 플러그인 기본값은 제한이 없어, 연결이 멈추면 상태가 `Checking`/`Downloading` 에 머물러 이후 확인이 모두 건너뛰어지고 트레이 항목이 계속 비활성이 된다. `UpdaterBuilder::timeout` 은 **확인 요청에만** 적용된다: `tauri-plugin-updater 2.14.1` 의 `update_from_release`(`updater.rs:735`)가 `Update.timeout` 을 `None` 으로 만들고 `download`(`:849`)는 그 필드만 읽으므로, `check()` 결과에 `update.timeout = Some(10분)` 을 직접 넣는다. 본문은 메모리 버퍼에 통째로 쌓이므로(`:869-874`) 크기 상한 64 MiB 를 둔다(응답의 `Content-Length` 가 넘거나 받은 양이 넘으면 중단. `on_chunk` 는 반환값이 없어 중단할 수 없으므로 `tokio::select!` 로 내려받기 future 를 버린다).
- "종료"로 설치할 때는 `restart_after_install(false)` 라 다시 실행하지 않는다. 설치본을 실행한 뒤 플러그인이 `std::process::exit(0)` 을 부르므로 `RunEvent::Exit` 가 오지 않아, 플러그인의 `on_before_exit` 훅에서 헬퍼를 먼저 내린다. 플러그인의 순서는 압축 풀기 → `on_before_exit` → `ShellExecuteW` → `exit(0)` 이다(`updater.rs:1017-1047`). **훅이 돈 뒤 `ShellExecuteW` 가 실패하면** 헬퍼가 내려가고 앱 정리까지 끝난 앱이 남으므로, 훅에서 세운 플래그로 알아내 "다시 시작하여 업데이트"는 `app.restart()`, "종료"는 `app.exit(0)` 으로 복구한다(서명 검증 실패 등 훅 전의 오류는 앱이 멀쩡해 그대로 둔다).
- 디버그 빌드, 버전 `0.0.0`(`0.0.0-*`), 서명 공개키가 없는 빌드는 업데이트를 켜지 않는다(플러그인도 등록하지 않는다 — 등록하면 설정에 `endpoints`·`pubkey` 가 있어야 앱이 시작된다).
- **공개키는 저장소의 `src-tauri/updater.pub` 파일**(공개 값, 커밋한다)에서만 읽는다. 저장소의 `tauri.conf.json` 에는 `plugins.updater` 도 `createUpdaterArtifacts` 도 없고, `npm run dist` 가 `scripts/tauri-build.mjs` 로 파일이 있을 때만 `release-tauri/updater.conf.json`(공개키·엔드포인트·passive·`requireSignedVersion`)을 만들어 `tauri build --config` 로 얹는다. **파일이 없으면 업데이터 없는 빌드**다. 개인키로 서명된 설치본이 필요한 로컬 빌드는 `TAURI_SIGNING_PRIVATE_KEY`(+`_PASSWORD`)가 있을 때만 `createUpdaterArtifacts` 를 켜 한 번에 서명하고, CI 는 항상 서명을 `sign` 잡으로 분리한다. 개인키·비밀번호는 저장소에 두지 않는다.
  - 키 만들기(한 번): `npx tauri signer generate -w <경로>\peekaboo-updater.key`(비밀번호 프롬프트는 Enter 로 비운다). 공개키 파일 `<…>.key.pub` 를 `src-tauri/updater.pub` 로 복사해 커밋한다. 개인키는 `gh secret set TAURI_SIGNING_PRIVATE_KEY < <개인키 파일>`(표준 입력으로, 명령줄에 싣지 않는다)로 저장소 시크릿에 넣는다. 비밀번호를 걸었다면 `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 도 넣는다.
  - `update-feeds.mjs` 는 피드를 쓰기 전에 서명을 직접 검증한다(`scripts/updater-sig.mjs`): `.sig` 의 키 ID 가 `updater.pub` 와 같은지, 설치본에 대한 Ed25519 서명(minisign `ED` = BLAKE2b-512 프리해시)이 맞는지, 전역 서명이 신뢰 주석을 덮는지, 그 안의 `version:` 이 릴리스 버전과 정확히 같은지. 하나라도 어긋나면 중단한다.
- 서명 설정에 `requireSignedVersion: true` 를 넣는다. 피드(`latest.json`)는 서명되지 않아, 없으면 피드를 바꿔치는 쪽이 새 버전 번호에 옛(유효하게 서명된) 설치본을 붙여 다운그레이드시킬 수 있다. tauri CLI 가 만드는 서명의 신뢰 주석에는 `version:` 이 들어 있다(실제 빌드의 `.sig` 로 확인).
- JS 에는 업데이터·알림 권한을 주지 않는다(`capabilities/preview.json` 에 `updater:*`·`notification:*` 없음). 플러그인이 등록하는 JS 커맨드는 ACL 에 막힌다.

## 7. 이득과 비용

| 지표 | Electron(실측, 2026-11-13) | Tauri(M0 실측·추정 구분 표기) |
|---|---|---|
| 설치본 | 128MB(실측 122.2MiB) | 약 8.6MiB(**추정**: exe+헬퍼 LZMA 압축) |
| 설치 폴더 | 379MB(실측 378MiB) | 약 23MiB(WebView2 는 OS 공용) |
| 업데이트 내려받기 | 최대 약 128MB | 약 8.6MiB(**추정**, 설치본과 같다고 봄) |
| 유휴 메모리 | Private 274.3MiB(M0 실측) | **실측** 252.8MiB, 절감 21.5MiB(처음 추정 80~150MB 보다 작다: WebView2 browser·crashpad·storage 프로세스가 Node main 몫을 대부분 채운다) |
| Space→표시 지연 | 로그 `shown in …ms` | 차이를 판별하지 못함(실측 불가) |

**잃는 것**: Chromium 버전 고정(사용자 PC 의 Evergreen WebView2 에 의존, 코덱은 Edge 미디어 스택), 알림 클릭 설치, 종료 시 자동 설치, `-webkit-app-region` 드래그. Electron 이 그냥 주던 기능을 COM 으로 직접 구현하므로 Rust·webview2-com 지식이 유지보수 요건이 된다.

**비용**: main 약 1,300줄 재작성(index·preview-window·protocol·helper-supervisor·native-preview·tray·updater·settings·logger), COM 연동, NSIS 훅, CI 시간 증가, 서명 키 관리.

**보류 기준**:

1. M0 판정 1~4 중 하나라도 대안 두 가지로 풀리지 않으면 보류한다.
2. M0 실측 메모리 절감이 50MB 미만이면 재평가한다. 대안은 Electron 유지 + 패키지 경량화. (M0 실측 21.5MiB 로 해당했고, 2026-11-13 사용자가 설치·업데이트 크기 이득을 근거로 계속하기로 결정했다. "M0 판정 결과" 참고)
3. M6 베타에서 포커스·Office 회귀가 풀리지 않으면 전환 PR 을 머지하지 않는다.

## 8. 위험(⚠ = 되돌리기 어려움)

1. ⚠ 전환 릴리스 이후 Tauri 사용자를 Electron 으로 자동으로 되돌릴 경로가 없다. M6 베타, 지켜보는 시간에 수동 발행, M8 전까지 Electron 코드 보존으로 완화한다.
2. ⚠ 업데이터 서명 키 분실·유출.
3. ⚠ 설치기 훅의 옛 제거 프로그램 실행·Run 값 재작성은 사용자 PC 에 남는 변경이라 리허설을 먼저 한다.
4. ⚠ 브리지가 없으므로 Electron 사용자는 직접 설치하기 전까지 옛 버전에 머문다.
5. 포커스·활성화(3.4), 네이티브 핸들러 z-order(3.1), 응답 버퍼링에 따른 메모리 튐(3.2), spare 창 효과 소실, 혼합 DPI.

## 9. 미확인 항목(실측으로 확인)

WebView2 클릭·드래그·드롭다운의 포커스 동작, 네이티브 핸들러 z-order, sandbox iframe 안의 IPC 주입 무력화, UNC 차단,
Tauri 크기·메모리, 렌더러 프로세스 공유, 여러 멤버 gzip 의 `DecompressionStream` 처리, 업데이터 설치 후 실제 재실행(인자는 근거를 찾았고 리허설로 확인),
`CheckIfAppIsRunning` 과 종료 중인 Electron 의 상호작용, CI 러너에서 GUI 캡처, Tauri NSIS 의 `-beta.N` 버전 처리.
