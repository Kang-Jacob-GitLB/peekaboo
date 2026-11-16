# Peekaboo — Claude Code 작업 지침

Windows 탐색기에서 파일 선택 후 Space 로 미리보기를 띄우는 상주 앱(Quick Look 류).
사용법·빌드 명령·구조 개요는 [README.md](README.md), 헬퍼 IPC 규격은 [docs/ipc.md](docs/ipc.md) 를 본다.

## 소통

- 사용자에게는 **한국어**로 답한다. 코드 주석·커밋 메시지·PR 본문도 한국어로 쓴다.

## 구성

- `helper/` — C# .NET 8 헬퍼(CsWin32). 키보드 훅(`Input/`), 탐색기 선택 조회·네이티브 미리보기 핸들러(`Shell/`).
  trimmed single-file `peekaboo-helper.exe` 로 publish 된다. 새 Win32 API 는 `NativeMethods.txt` 에 추가한다.
- `src-tauri/` — 호스트인 Tauri 2([docs/tauri-migration.md](docs/tauri-migration.md), 옛 Electron 호스트는 M8 에서 제거). `release.yml` 이 이 설치본(build → sign → release)을 발행한다.
  `helper.rs`·`session.rs`·`controller.rs`(헬퍼 감시와 메시지 처리), `peek.rs`(프로토콜), `preview.rs`(창), `guard.rs`(WebView2 보호), `commands.rs`(렌더러 커맨드, ACL 은 `build.rs`·`capabilities/`).
  렌더러 어댑터는 `src/renderer/host/tauri.ts`, 렌더러 빌드는 `vite.config.ts`(`npm run build` → `out/renderer`, `tauri.conf.json` 의 `frontendDist`).
  트레이(`tray.rs`)·설정(`settings.rs`)·시작 시 실행 Run 키(`startup.rs`)·로그(`logger.rs`)·데이터 폴더(`appdata.rs`)는 옛 Electron 설치본과 같은 위치·형식을 써서 설정이 이어진다.
  **디버그 빌드의 데이터 폴더는 `%APPDATA%\Peekaboo-dev`** 이고 시작 시 실행 메뉴는 꺼져 있다 — 설치된 Peekaboo 의 `settings.json`·로그·Run 값을 개발 중에 읽거나 쓰지 않는다.
  상주 모드(환경 변수 없는 실행)는 트레이를 만들므로 시험은 순수 함수 테스트 또는 `PEEKABOO_PREVIEW`+`PEEKABOO_OFFSCREEN=1` 로 한다.
  설치본은 `npm run dist`(헬퍼를 `src-tauri/binaries/` 로 복사 → `tauri build` → `release-tauri/`). `externalBin` 은 `tauri.bundle.conf.json` 에만 두어 일반 `cargo` 빌드가 헬퍼 없이도 되게 한다.
  설치본의 exe 는 `mainBinaryName` 으로 `Peekaboo.exe`(옛 Electron 설치본과 같은 이름)가 되고, 일반 `cargo` 빌드의 산출물은 `peekaboo.exe` 다. NSIS 훅은 `src-tauri/windows/hooks.nsh`.
- `src/renderer/plugins/` — 포맷별 렌더러. `PreviewPlugin` 을 구현한 모듈 하나 + `registry.ts` 한 줄로 새 포맷을 추가한다.
  `registry.ts` 의 순서가 곧 우선순위다.
- `src/renderer/host/` — 렌더러 ↔ 호스트 어댑터(`createHost()`). 플러그인은 호스트 API 를 `ctx.host` 로 받는다.
  텍스트 디코딩·압축 목록 해석은 `file-ops/` 가 `peek` 프로토콜 Range 요청으로 읽어 렌더러에서 한다(호스트에 파서를 두지 않는다).

## 규칙

- **COM/Win32 API 는 추측하지 않는다.** 시그니처가 불확실하면 Microsoft Learn 을 확인한 뒤 쓴다.
- QuickLook(QL-Win/QuickLook, GPL-3.0) 코드나 기법을 가져오면 해당 파일 머리말에 출처(`QL-Win/QuickLook@<SHA>`)와 변경 사실을 적는다.
  라이선스는 GPL-3.0-only 다.
- 렌더러에서 쓰는 라이브러리는 `devDependencies` 에 둔다(vite 가 번들). Node 쪽에서 실행 중 쓰는 런타임 의존성은 없으므로 `dependencies` 는 비워 둔다.
- 새 서드파티 라이브러리를 넣으면 README 의 서드파티 라이선스 표에 추가한다.
- 버전은 태그로만 관리한다. `package.json`·`helper/Peekaboo.Helper.csproj` 의 `0.0.0` 은 건드리지 않는다.
- 줄바꿈은 LF 를 유지한다(스크립트로 파일을 고칠 때 CRLF 로 바뀌지 않게 주의).

## 보안 경계 (미리보기 대상 파일은 신뢰하지 않는다)

- 렌더러(WebView2)가 부를 수 있는 것은 `build.rs` 의 `COMMANDS` 와 `capabilities/preview.json` 이 preview 창에 준 권한뿐이다(`core:window` 등 창 제어 권한은 없다). CSP 는 `tauri.conf.json` 의 `app.security.csp`(`index.html` 에는 두지 않는다).
  창 이동·새 창은 호스트가 막는다: 메인 프레임은 앱 출처 외로 이동할 수 없고(`preview.rs` 의 `on_navigation`·`is_app_url`), 새 창은 거부한다(`on_new_window`).
- `peek` 프로토콜·`list_dir` 등은 **현재 미리보기 중인 경로 하나만** 허용한다(`PreviewState::is_allowed`, 정확히 일치. `peek.rs`·`commands.rs` 가 호출). 커맨드는 호출한 웹뷰가 preview 창인지도 다시 검사한다.
  `peek` 응답에는 `sandbox; default-src 'none'` CSP 를 붙인다(응답이 문서로 열려도 스크립트·앱 출처 접근 불가).
- HTML 미리보기는 `allow-scripts` 없는 sandbox iframe. 서브프레임 이동은 `guard.rs` 의 `FrameNavigationStarting`(`about:` 외 취소)이 막고,
  `file://`(UNC 포함) 같은 외부 리소스 요청은 앱 CSP 에 `file:` 출처가 없어 막힌다.
- Markdown·docx 의 HTML 은 DOMPurify 로 정화한다. 이 경계를 넓히는 변경은 보안 리뷰를 거친다.

## README

- **README.md 는 300줄 이내로 유지한다.** 넘칠 것 같으면 세부 내용을 `docs/` 로 옮기고 링크한다.
  PR 검사(`build.yml` 의 README 길이 점검)가 300줄을 넘으면 실패시킨다.
- 지원 포맷·알려진 제한이 바뀌면 README 도 같은 PR 에서 고친다.

## 브랜치·PR·릴리스

- 코드 변경은 워크트리에서, PR 로만 넣는다. `main` 은 룰셋으로 보호된다(PR 필수, `Typecheck · Build` 통과, 선형 이력).
- 머지는 **항상 squash**(`gh pr merge --squash`).
- **PR 제목은 Conventional Commits** — squash 커밋 제목이 되고, 매일 00:00 KST 자동 릴리스가 이것으로 버전을 정한다.
  `feat` → minor, `fix`·`perf`·`style` → patch, `!`/`BREAKING CHANGE` → major, `docs`·`build`·`ci`·`chore`·`test`·`refactor` → 릴리스 없음.
- 즉시 릴리스: `gh workflow run nightly-release.yml`. 다음 버전 미리 보기: `npm run next-version`.
- 커밋 전에 비밀값(키·토큰·개인 경로·회사 정보)이 없는지 확인한다.

## 확인 방법

- 기본 검증: `npm run typecheck` → `npm test` → `npm run build`(헬퍼를 고쳤으면 `npm run helper:build`, 호스트를 고쳤으면 위 Rust 검사).
- 미리보기 화면 확인은 키 입력 없이, **화면 밖 모드(`PEEKABOO_OFFSCREEN=1`)를 기본으로** 한다. 사용자가 PC 를 쓰는 중에도 창이
  어느 모니터에도 뜨지 않고 포커스를 가져가지 않는다(창을 모든 모니터 왼쪽 바깥에 두고, Chromium 이 가려진 창의 렌더를 멈추지 않게 하며,
  헬퍼도 띄우지 않고(`PEEKABOO_HELPER` 가 없으면), 캡처가 끝나면 스스로 종료한다). 전체 화면 스크린샷은 찍지 않고 **미리보기 창만** 캡처한다.
- 캡처: `npm run build` → `cargo build`(`src-tauri/`) 후
  `PEEKABOO_HOTKEY=0 PEEKABOO_OFFSCREEN=1 PEEKABOO_PREVIEW=<파일> PEEKABOO_CAPTURE=<png> src-tauri/target/debug/peekaboo.exe`
  (디버그 빌드 전용, 캡처 뒤 스스로 종료. 디버그 빌드는 `Peekaboo-dev` 데이터 폴더를 써서 설치본의 설정과 섞이지 않는다).
  Rust 검사는 `cargo fmt --check`·`cargo clippy --locked --all-targets -- -D warnings`·`cargo test --locked`.
  화면 안에 띄우는 확인(`PEEKABOO_OFFSCREEN` 없이)은 사용자가 PC 를 쓰지 않을 때만 한다. 화면 밖 모드는 창이 모니터와 겹치면 띄우지 않고 종료한다.
- 메모리 실측도 같은 모드로 한다(호스트 프로세스의 Private·PeakWorkingSet 을 폴링). Office 미리보기 핸들러는 Word/Excel 을 기동하므로 PC 를 쓰는 중에는 시험하지 않는다.
- 키를 보내는 GUI 테스트는 사용자가 PC 를 쓰지 않을 때만 한다. 전경 창이 예상한 탐색기 창이 아니면(잠금 화면 포함) 즉시 중단한다.
- 로그: `%APPDATA%\Peekaboo\logs\peekaboo.log`(표시 지연, 헬퍼 상태, 업데이트 진행).
