# Peekaboo — Claude Code 작업 지침

Windows 탐색기에서 파일 선택 후 Space 로 미리보기를 띄우는 상주 앱(Quick Look 류).
사용법·빌드 명령·구조 개요는 [README.md](README.md), 헬퍼 IPC 규격은 [docs/ipc.md](docs/ipc.md) 를 본다.

## 소통

- 사용자에게는 **한국어**로 답한다. 코드 주석·커밋 메시지·PR 본문도 한국어로 쓴다.

## 구성

- `helper/` — C# .NET 8 헬퍼(CsWin32). 키보드 훅(`Input/`), 탐색기 선택 조회·네이티브 미리보기 핸들러(`Shell/`).
  trimmed single-file `peekaboo-helper.exe` 로 publish 된다. 새 Win32 API 는 `NativeMethods.txt` 에 추가한다.
- `src/main/` — Electron main. 헬퍼 감시(`helper-supervisor.ts`), 미리보기 창(`preview-window.ts`),
  `peek://` 프로토콜과 요청 차단(`protocol.ts`), 트레이, 자동 업데이트(`updater.ts`).
- `src/renderer/plugins/` — 포맷별 렌더러. `PreviewPlugin` 을 구현한 모듈 하나 + `registry.ts` 한 줄로 새 포맷을 추가한다.
  `registry.ts` 의 순서가 곧 우선순위다.

## 규칙

- **COM/Win32 API 는 추측하지 않는다.** 시그니처가 불확실하면 Microsoft Learn 을 확인한 뒤 쓴다.
- QuickLook(QL-Win/QuickLook, GPL-3.0) 코드나 기법을 가져오면 해당 파일 머리말에 출처(`QL-Win/QuickLook@<SHA>`)와 변경 사실을 적는다.
  라이선스는 GPL-3.0-only 다.
- 렌더러에서 쓰는 라이브러리는 `devDependencies` 에 둔다(vite 가 번들). main 이 실행 중 `require` 하는 것만 `dependencies`.
- 새 서드파티 라이브러리를 넣으면 README 의 서드파티 라이선스 표에 추가한다.
- 버전은 태그로만 관리한다. `package.json`·`helper/Peekaboo.Helper.csproj` 의 `0.0.0` 은 건드리지 않는다.
- 줄바꿈은 LF 를 유지한다(스크립트로 파일을 고칠 때 CRLF 로 바뀌지 않게 주의).

## 보안 경계 (미리보기 대상 파일은 신뢰하지 않는다)

- 렌더러는 `sandbox`·`contextIsolation`, CSP 는 `src/renderer/index.html`. 창 이동·새 창은 main 이 막는다.
- `peek://`·`readText` 등은 **현재 미리보기 중인 경로 하나만** 허용한다(`isAllowedPath`, 정확히 일치).
- HTML 미리보기는 `allow-scripts` 없는 sandbox iframe. 서브프레임 이동은 `will-frame-navigate`, UNC(`file://호스트/`) 요청은 `blockRemoteFileRequests` 가 막는다.
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

- 기본 검증: `npm run typecheck` → `npm run build`(헬퍼를 고쳤으면 `npm run helper:build`).
- 미리보기 화면 확인은 키 입력 없이 한다: `npm run build` 후
  `PEEKABOO_PREVIEW=<파일> PEEKABOO_CAPTURE=<png> electron .` 로 **미리보기 창만** 캡처한다. 전체 화면 스크린샷은 찍지 않는다.
  설치된 Peekaboo 가 실행 중이면 `--user-data-dir=<임시 폴더>` 를 붙여 단일 인스턴스 잠금을 피한다.
- 키를 보내는 GUI 테스트는 사용자가 PC 를 쓰지 않을 때만 한다. 전경 창이 예상한 탐색기 창이 아니면(잠금 화면 포함) 즉시 중단한다.
- 로그: `%APPDATA%\Peekaboo\logs\peekaboo.log`(표시 지연, 헬퍼 상태, 업데이트 진행).
