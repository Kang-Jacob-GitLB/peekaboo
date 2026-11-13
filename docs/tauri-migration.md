# Electron → Tauri 2 전환 계획

> 상태: **계획**(2026-11-13). 아직 아무것도 구현·실측하지 않았다. Tauri 쪽 내용은 공식 문서(v2.tauri.app, docs.rs)와
> wry·tao 소스, Microsoft Learn 을 읽고 판단한 것이다. 이 계획 전체가 M0 시험 구현의 판정에 걸려 있다.
> 근거 표기 `파일:줄` 은 이 문서를 쓴 시점(67710ca)의 저장소 기준이다.

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
| WebView2 런타임 | 내려받기 방식(`downloadBootstrapper`, 설치본 크기 증가 없음) |
| Electron → Tauri 업데이트 브리지 | **두지 않음.** 사용자가 직접 설치, 설치기가 옛 버전 제거 |
| 알림 클릭으로 업데이트 설치 | 포기. 알림은 안내만 하고 트레이 "다시 시작하여 업데이트"로 설치 |
| 업데이터 서명 키 | GitHub Actions 시크릿 + 오프라인 백업. 키 생성은 저장소 관리자가 직접 |
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
| sandbox iframe (`src/renderer/plugins/html/index.ts:24`) | Windows 에서는 초기화 스크립트가 서브프레임에도 주입된다. 스크립트가 꺼진 프레임에서는 실행되지 않을 것으로 보지만 **M0 에서 실측** |
| `will-frame-navigate`, 새 창 차단 | `FrameNavigationStarting` 취소, wry 기본 거부 |
| UNC 차단 | **실측 전에는 막힌다고 단정하지 않는다.** HTML·Markdown 의 `<img src="file://호스트/…">` 로 SMB 연결이 생기는지 확인하고, 새면 `WebResourceRequested` 로 403 |
| 클립보드 | 우클릭은 호스트 이벤트, Ctrl+C 는 기존 `pendingCopy` id 가드를 Rust 로. 렌더러가 임의 문자열을 쓰는 커맨드는 만들지 않는다 |

isolation pattern 은 쓰지 않는다. 이 앱의 위협은 프런트엔드 의존성이 아니라 미리볼 파일이고, 그 내용은 스크립트를 실행할 수 없는 구조다.

### 3.4 미리보기 창 동작

창 상태 기계(`PreviewWindow`)를 Rust 로 1:1 이식하되, 포커스 관련 동작은 HWND 직접 제어 경로를 준비한다.

- **두 번째 이후 show**: tao `show()` 는 `MARKER_DONT_FOCUS` 가 있을 때만 `SW_SHOWNOACTIVATE`. 같은 창을 hide→show 반복하므로(`preview-window.ts:162-164,277`) 핵심 시험 항목이다. 대안은 `SetWindowPos(HWND_TOPMOST, …, SWP_NOACTIVATE|SWP_SHOWWINDOW)` / `ShowWindow(SW_HIDE)` 직접 호출.
- **클릭**: WebView2 콘텐츠 창은 다른 프로세스(msedgewebview2)의 자식이라, 비활성 창 클릭 시 활성화 여부를 실측해야 한다.
- **`<select>` 드롭다운**: 팝업이 msedgewebview2 소유 최상위 창이라 헬퍼가 "다른 앱 전환"으로 오인할 수 있다. `config.hostPids`(앱 PID + `BrowserProcessId`)를 추가 필드로 보낸다. 바뀌는 곳: `helper/HelperCore.cs`, `helper/Input/FocusInspector.cs`, `helper/Ipc/Messages.cs`, `src/shared/ipc-types.ts`, `docs/ipc.md`.
- **헤더 드래그**: WebView2 `IsNonClientRegionSupportEnabled` 는 시스템 메뉴·더블클릭 최대화가 함께 켜지고 사용자 이동을 구분할 수 없어 쓰지 않는다.
- **spare 창**: 그대로 이식하되 Windows 에서는 동기 커맨드·이벤트 핸들러 안에서 창을 만들면 교착하므로 항상 async 태스크에서. WebView2 가 렌더러 프로세스를 공유하면 교체 효과가 없을 수 있어 M3 에서 확인한다.
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
| **M4** 트레이·설정·패키징 | 트레이, 설정, Run 키, 로그, single-instance, `externalBin`, NSIS `currentUser`, `release.yml` 에 수동 실행 전용 `build-tauri` 잡(발행 안 함) | 깨끗한 계정에 아티팩트 설치 |
| **M5** 업데이터·옛 버전 제거 | `tauri-plugin-updater`, `src-tauri/windows/hooks.nsh`(6절), `latest.json` 생성, 서명 키 시크릿 | 옛 Electron 버전 3개(0.2.0, 중간, 최신) 위에 Tauri 설치본 설치 리허설, Tauri → Tauri 업데이트는 테스트 저장소에서 |
| **M6** 베타 | 프리릴리스 태그를 수동 발행, 테스터가 직접 설치(Electron 사용자는 `allowPrerelease=false` 라 받지 않음, `updater.ts:54`) | 1~2주 실사용: 포커스 회귀, Office 핸들러, 메모리 |
| **M7** 전환 | `feat!: Tauri 2 로 전환`. `release.yml` 기본 빌드를 Tauri 로, README·CLAUDE.md 갱신 | 사람이 지켜보는 시간에 `gh workflow run nightly-release.yml` |
| **M8** 정리 | `src/main`, `src/preload`, `electron-builder.yml`, `electron.vite.config.ts`, electron 의존성 제거 | typecheck, build, cargo test |

### M0 판정 항목

하나라도 실패하고 대안 두 가지(tao API → HWND 직접 제어 → B안)로도 풀리지 않으면 **전환을 보류한다.**

1. 탐색기가 포그라운드일 때 Space → Esc → Space 로 **두 번째 표시**까지 탐색기 포커스가 유지되고 방향키로 다음 파일로 넘어간다.
2. 본문 클릭, 드래그 선택, 휠 스크롤, ✕, 헤더 드래그 뒤에도 탐색기 포커스가 유지된다.
3. raw 플러그인 `<select>` 를 연 뒤에도 포커스가 유지되고 헬퍼가 다른 앱 전환으로 오인하지 않는다.
4. Office 핸들러 창이 깜박임 없이 WebView2 위에 겹치고 리사이즈를 따라간다.
5. `peek` 로 영상을 재생·seek 할 수 있고 100MB PDF 가 열린다.
6. sandbox iframe 안에서 IPC 객체를 쓸 수 없고 스크립트가 실행되지 않는다. `file://호스트/` 요청이 나가지 않는다.
7. 실측: 설치본·설치 폴더 크기, 유휴 메모리(프로세스별), 콜드 스타트, Space→표시 지연.

### 캡처 방식

`npm run build:web:tauri` → `cargo build` → `PEEKABOO_PREVIEW=<파일> PEEKABOO_CAPTURE=<png> src-tauri/target/debug/peekaboo.exe`.
이 모드에서는 single-instance 를 등록하지 않고, WebView2 데이터 폴더를 임시 폴더로 바꾸고, 헬퍼는 `PEEKABOO_HELPER=1` 일 때만 띄운다(키보드 훅 중복 방지).
CapturePreview 는 웹 콘텐츠만 찍는다(Office 네이티브 창 제외, `capturePage` 와 같음).

## 5. CI·릴리스

- **`build.yml`**: 필수 체크 job 이름 `Typecheck · Build` 를 유지하고 같은 job 에 Rust 툴체인·캐시(SHA 고정), `npm run build:web:tauri`, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --locked` 를 추가한다. job 을 나누면 룰셋의 required check 를 바꿔야 한다. PR 시간은 콜드 캐시 +8~15분, 웜 캐시 +2~5분으로 추정.
- **`release.yml`**: `npm pkg set version` 유지, `tauri.conf.json` 의 `version` 이 `../package.json` 을 가리키게 하고 `Cargo.toml` 은 `0.0.0` 고정. 헬퍼 산출물을 `src-tauri/binaries/peekaboo-helper-x86_64-pc-windows-msvc.exe` 로 복사, `npx tauri build --bundles nsis`, 서명 키는 시크릿 `TAURI_SIGNING_PRIVATE_KEY`(+`_PASSWORD`). 산출물 이름 `Peekaboo-Setup-${version}-x64.exe` 유지. 메타데이터 확인 단계는 `latest.json`(버전, 서명, URL) 검사로 바꾼다.
- **`nightly-release.yml`**: 변경 없음.
- **서명**: 업데이터 minisign 키는 필수이고 끌 수 없다. **키를 잃으면 기존 Tauri 설치본은 다시는 자동 업데이트를 받지 못한다.** 시크릿 + 오프라인 백업. Authenticode 는 지금처럼 없다.
- **로컬 개발**: README "필요한 도구"에 Rust(MSVC) 추가.

## 6. 기존 Electron 사용자 이전

**자동 업데이트 브리지를 두지 않는다.** Tauri 릴리스는 `latest.yml` 을 발행하지 않으므로, Electron 설치본은 마지막 Electron 버전에 머물고 업데이트 확인은 실패로 끝난다. 사용자는 새 설치본을 직접 받아 설치한다.

Tauri NSIS 는 설치 경로(`%LOCALAPPDATA%\Peekaboo`)와 제거 키(`HKCU\…\Uninstall\Peekaboo`)가 Electron(`%LOCALAPPDATA%\Programs\Peekaboo`, 제거 키 GUID `a32366e0-3e8a-5400-a683-97d78e557198`)과 달라서,
그냥 설치하면 두 앱이 함께 떠 키보드 훅이 2개가 되고 Space 가 두 번 처리된다. 그래서 설치기 훅(`src-tauri/windows/hooks.nsh`)이 옛 버전을 정리한다.

1. **PREINSTALL**: HKCU 옛 제거 키에서 `UninstallString` 을 읽고, Run 값(`io.github.kang-jacob-gitlb.peekaboo`)이 있었는지 기록한 뒤, 옛 제거 프로그램을 `/S` 로 실행하고 기다린다. 옛 제거 프로그램이 같은 이름의 바로가기를 지우므로 새 바로가기를 만들기 **전에** 한다.
2. **POSTINSTALL**: Run 값이 있었으면 같은 이름으로 `"$INSTDIR\Peekaboo.exe"` 를 다시 쓴다(작업 관리자의 "사용 안 함" 상태도 이어진다).
3. "모든 사용자"로 설치한 옛 버전(HKLM)은 관리자 권한 없이 제거할 수 없어 건드리지 않는다. 처리 방식은 M5 에서 정한다.
4. 실행 중 앱 검사가 종료 중인 옛 `Peekaboo.exe` 를 잡아 설치가 멈추는지 리허설로 확인한다.

설정과 로그는 `%APPDATA%\Peekaboo\{settings.json, logs\peekaboo.log}` 를 그대로 쓴다. WebView2 데이터는 Tauri 기본 위치에 둔다.

**Tauri 업데이터**: 엔드포인트 `https://github.com/Kang-Jacob-GitLB/peekaboo/releases/latest/download/latest.json`. 10초 뒤 한 번, 이후 6시간마다 확인하고 내려받아 두었다가 트레이 "다시 시작하여 업데이트" 또는 "종료"에서 설치한다. OS 종료·로그오프 때 조용한 설치는 보장하지 못한다(지금의 `autoInstallOnAppQuit` 보다 약함). 설치 후 재실행 인자는 미확인이라 M5 리허설 항목이다.

## 7. 이득과 비용

| 지표 | Electron(실측, 2026-11-13) | Tauri(추정, M0 에서 실측) |
|---|---|---|
| 설치본 | 128MB | 약 8~15MB |
| 설치 폴더 | 379MB | 약 25~40MB(WebView2 는 OS 공용) |
| 업데이트 내려받기 | 최대 약 128MB | 약 10MB |
| 유휴 메모리 | Electron 4개 프로세스 Private 합 약 289MB + 헬퍼 32MB | 같은 Chromium 프로세스 구조라 절감은 Node main 몫 약 80~150MB 로 추정 |
| Space→표시 지연 | 로그 `shown in …ms` | 비슷할 것으로 봄 |

**잃는 것**: Chromium 버전 고정(사용자 PC 의 Evergreen WebView2 에 의존, 코덱은 Edge 미디어 스택), 알림 클릭 설치, 종료 시 자동 설치, `-webkit-app-region` 드래그. Electron 이 그냥 주던 기능을 COM 으로 직접 구현하므로 Rust·webview2-com 지식이 유지보수 요건이 된다.

**비용**: main 약 1,300줄 재작성(index·preview-window·protocol·helper-supervisor·native-preview·tray·updater·settings·logger), COM 연동, NSIS 훅, CI 시간 증가, 서명 키 관리.

**보류 기준**:

1. M0 판정 1~4 중 하나라도 대안 두 가지로 풀리지 않으면 보류한다.
2. M0 실측 메모리 절감이 50MB 미만이면 재평가한다. 대안은 Electron 유지 + 패키지 경량화.
3. M6 베타에서 포커스·Office 회귀가 풀리지 않으면 전환 PR 을 머지하지 않는다.

## 8. 위험(⚠ = 되돌리기 어려움)

1. ⚠ 전환 릴리스 이후 Tauri 사용자를 Electron 으로 자동으로 되돌릴 경로가 없다. M6 베타, 지켜보는 시간에 수동 발행, M8 전까지 Electron 코드 보존으로 완화한다.
2. ⚠ 업데이터 서명 키 분실·유출.
3. ⚠ 설치기 훅의 옛 제거 프로그램 실행·Run 값 재작성은 사용자 PC 에 남는 변경이라 리허설을 먼저 한다.
4. ⚠ 브리지가 없으므로 Electron 사용자는 직접 설치하기 전까지 옛 버전에 머문다.
5. 포커스·활성화(3.4), 네이티브 핸들러 z-order(3.1), 응답 버퍼링에 따른 메모리 튐(3.2), spare 창 효과 소실, 혼합 DPI.

## 9. 미확인 항목(실측으로 확인)

WebView2 클릭·드래그·드롭다운의 포커스 동작, 네이티브 핸들러 z-order, sandbox iframe 안의 IPC 주입 무력화, UNC 차단,
Tauri 크기·메모리, 렌더러 프로세스 공유, 여러 멤버 gzip 의 `DecompressionStream` 처리, 업데이터 설치 후 재실행,
`CheckIfAppIsRunning` 과 종료 중인 Electron 의 상호작용, CI 러너에서 GUI 캡처, Tauri NSIS 의 `-beta.N` 버전 처리.
