<p align="center"><img src="resources/icon.png" width="128" alt="Peekaboo 아이콘"></p>

<h1 align="center">Peekaboo</h1>

Windows 탐색기에서 파일을 선택하고 **스페이스바**를 누르면 바로 미리보기 창이 뜨는 상주 프로그램입니다.
macOS Quick Look 과 비슷한 사용감을 목표로 합니다. 일부 구현은 [QuickLook](https://github.com/QL-Win/QuickLook)(GPL-3.0)을 참고했습니다(아래 라이선스 참고).

> 🚧 개발 중 (v0.1, MVP) — 아래 지원 포맷 중 체크된 것만 동작합니다.

<!-- 스크린샷 자리 -->
<p align="center"><em>(스크린샷 준비 중)</em></p>

## 사용법

| 키 | 동작 |
|---|---|
| Space | 미리보기 열기 / 닫기 |
| Esc | 닫기 |
| ↑ ↓ ← → | 탐색기 선택을 따라 이전/다음 파일 미리보기 |
| Enter | 기본 앱으로 열기 |

탐색기 창 또는 바탕화면의 파일 목록에 포커스가 있을 때만 동작합니다. 이름 바꾸기, 주소창, 검색창에 입력 중일 때는 스페이스바를 가로채지 않습니다.
미리보기 창은 내용 비율에 맞춰 뜹니다: 사진·영상·PDF 는 원본 비율, Word 는 A4 세로, PowerPoint 는 16:9(화면의 85% 이내), Markdown·HTML 은 기본 크기.
Markdown·HTML 의 제목, PDF 의 책갈피가 2개 이상이면 왼쪽에 목차가 붙습니다(누르면 그 위치로 이동, « 로 접기).
다른 앱으로 전환하면 미리보기가 자동으로 닫힙니다(탐색기 창·바탕화면·작업 표시줄·시작 메뉴·화면 캡처로 옮겨 가거나 미리보기 창을 클릭할 때는 유지).
트레이 아이콘 메뉴에서 단축키 켜기/끄기, Windows 시작 시 실행을 설정할 수 있습니다.

## 지원 포맷

- [x] 이미지: jpg, png, gif, webp, svg, bmp, ico, avif
- [x] 헤더 없는 카메라 프레임(raw, yuv, nv12): 파일 크기로 해상도·포맷(NV12·NV21·I420·YUYV·UYVY·RGB·BGRA·Gray)을 추정, 오른쪽 위 선택 상자로 바꿔 보기.
  DSLR RAW 처럼 크기가 흔한 해상도와 맞지 않으면 폴백 카드로 표시
- [x] 텍스트 / 소스 코드 (구문 강조, UTF-8/UTF-16/CP949 자동 감지, 512KB까지)
  — 모르는 확장자라도 앞부분 4KB 가 텍스트로 보이면 텍스트로 표시(`.list`, 사내 설정 파일 등)
- [x] Markdown (GitHub 과 같은 모양, 체크박스·`> [!NOTE]` 알림·본문 속 HTML 지원, 상대 경로 이미지는 대체 텍스트로 표시, 제목 목차)
- [x] HTML (html·htm·xhtml 을 렌더링된 화면으로 표시, 2MB까지, 제목 목차)
  — 스크립트는 실행하지 않고, 외부(인터넷)·상대 경로 리소스는 불러오지 않으며, 링크는 열지 않습니다
- [x] PDF (pdf.js, 보이는 페이지만 렌더, 책갈피 목차)
- [x] 영상 / 오디오 (mp4·webm·mov·mkv, mp3·m4a·wav·ogg·opus·flac — Chromium 이 재생 가능한 코덱)
- [x] 폴더 (파일 목록, 최대 500개)
- [x] 압축 파일 목록 (zip·jar·apk·vsix·nupkg·whl·epub 등과 tar·tar.gz·tgz, 압축 해제 없이 목록만, ZIP64·CP949 파일명)
  — 7z·rar 는 지원하지 않습니다(7z 는 목록 자체가 LZMA 로 압축돼 있고, rar 는 비공개 형식)
- [x] Word / Excel / PowerPoint (docx·xlsx·pptx, 구형 doc·xls·ppt·rtf)
  — Windows 에 등록된 미리보기 핸들러(Office 설치 시 원본과 같은 모양)를 우선 사용하고,
  없으면 자체 간이 미리보기(docx 본문·표, xlsx 시트 표, pptx 슬라이드 텍스트·이미지)로 표시

## 설치

[Releases](../../releases) 에서 `Peekaboo-Setup-x.y.z-x64.exe` 를 받아 실행합니다.

> ⚠️ **SmartScreen 경고 안내** — 아직 코드 서명 인증서로 서명하지 않은 빌드라서, 처음 실행할 때
> "Windows의 PC 보호" 경고가 뜹니다. **추가 정보 → 실행** 을 누르면 설치됩니다.
> 불안하다면 아래 방법으로 소스에서 직접 빌드할 수 있습니다.

### 업데이트

0.2.0 부터는 앱이 스스로 업데이트합니다.

- 시작 10초 뒤와 이후 6시간마다 [Releases](../../releases) 에서 새 버전을 확인하고, 있으면 백그라운드로 내려받습니다.
- 다 받으면 알림이 뜹니다. 알림이나 트레이 메뉴의 **다시 시작하여 업데이트** 를 누르면 바로 설치·재실행하고,
  그냥 두면 Peekaboo 를 끌 때 조용히 설치됩니다.
- 트레이 메뉴의 **업데이트 확인** 으로 바로 확인할 수도 있습니다.
- 설치 파일은 릴리스의 `latest.yml` 에 적힌 SHA-512 와 맞는지 검사한 뒤 실행합니다.
- 0.1.0 에는 업데이트 기능이 없어 0.2.0 은 직접 받아 설치해야 합니다.

## 빌드

필요한 도구

- Node.js 22 이상
- .NET SDK 8 이상 (헬퍼는 `net8.0-windows` 를 타깃으로 하며, 더 높은 버전 SDK 로도 빌드됩니다)
- Windows 10 1809 이상 / Windows 11 (x64)

```powershell
npm install
npm run helper:build   # C# 헬퍼 → helper/publish/peekaboo-helper.exe
npm run dev            # 개발 모드 실행
npm run dist           # 설치 파일 → release/
```

### 단계별 확인 방법

헬퍼만 단독으로 시험하기 (Electron 없이):

```powershell
npm run helper:console
# 탐색기에서 파일 선택 → Space: 선택 경로 출력 / 방향키: 바뀐 선택 출력 / Esc: 닫힘
helper\publish\peekaboo-helper.exe --list   # 열린 탐색기 창·바탕화면의 선택을 한 번 출력
```

미리보기 창만 시험하기 (키 입력 없이, 개발 모드 전용):

```powershell
npm run build
$env:PEEKABOO_PREVIEW = "C:\work\notes.md"; npx electron .
```

로그(`%APPDATA%\Peekaboo\logs\peekaboo.log`)에 `shown in …ms`, `rendered … in …ms since key` 로 표시 지연이 기록됩니다.

## 기여·릴리스

- `main` 은 룰셋으로 보호됩니다: PR 필수, 필수 검사 `Typecheck · Build`(`.github/workflows/build.yml`) 통과, squash 머지만 허용.
- **PR 제목은 [Conventional Commits](https://www.conventionalcommits.org/ko/) 형식**으로 씁니다. squash 머지라 PR 제목이 곧 `main` 의 커밋 제목이고, 자동 릴리스가 이것으로 버전을 정합니다.

| PR 제목 | 다음 버전 |
| --- | --- |
| `feat: …` | minor (0.2.0 → 0.3.0) |
| `fix: …` · `perf: …` · `style: …` | patch (0.2.0 → 0.2.1) |
| `feat!: …` 또는 본문에 `BREAKING CHANGE:` | major |
| `docs:` · `build:` · `ci:` · `chore:` · `test:` · `refactor:` | 릴리스하지 않음 |

- **매일 00:00(KST)** `nightly-release.yml` 이 마지막 태그 이후 `main` 커밋을 보고, 위 표에서 릴리스할 변경이 있으면
  Windows 설치본을 빌드해 태그와 Release(설치본 + `latest.yml` + blockmap)를 만듭니다. 문서·빌드 설정만 바뀐 날은 건너뜁니다.
- 바로 릴리스하려면 Actions → `nightly-release` → **Run workflow**(올림 단계 선택, `dry_run` 으로 계산만 해 볼 수도 있음).
- 다음 버전 미리 보기: `npm run next-version`
- 버전은 태그로만 관리합니다. `package.json`·헬퍼 csproj 의 `0.0.0` 은 그대로 두세요(빌드할 때 태그 버전이 들어갑니다).

## 구조

```
Electron (TypeScript) ── 트레이, 설정, 미리보기 창, 포맷별 렌더러 플러그인
   ▲  stdin/stdout JSON Lines  (docs/ipc.md)
   ▼
peekaboo-helper.exe (C# .NET 8) ── 키보드 훅 + 탐색기 선택 조회 (COM)
```

- `helper/` — Win32/COM 연동 헬퍼. 탐색기에 포커스가 있을 때 키 입력을 감지하고 선택된 파일 경로를 조회합니다.
- `src/main/` — Electron main 프로세스. 헬퍼 감시/재시작, 트레이, 로그, 미리보기 창 관리.
- `src/renderer/plugins/` — 포맷별 렌더러. `PreviewPlugin` 인터페이스를 구현하고 `registry.ts` 에 등록하면 새 포맷이 추가됩니다.

로그는 `%APPDATA%\Peekaboo\logs\` 에 저장됩니다.

## 알려진 제한

- **관리자 권한으로 실행된 탐색기 창에서는 동작하지 않습니다.** 일반 권한 프로그램은 Windows 보안 정책(UIPI) 때문에 권한이 높은 창의 선택 항목을 읽을 수 없습니다. 이때는 키를 그대로 통과시키며 앱이 종료되지는 않습니다.
- 탐색기 이외의 파일 대화상자, 서드파티 파일 관리자는 지원하지 않습니다.
- Office 미리보기 핸들러는 처음 한 번 Office 프로그램이 백그라운드로 기동되느라 수 초 걸릴 수 있습니다. 8초 안에 뜨지 않으면 간이 미리보기로 대신 표시합니다.
- Office 미리보기는 별도 네이티브 창이라 미리보기 창 안에서 스크롤·클릭이 핸들러 동작을 따릅니다.
- 다크 모드에서도 Word·Excel·PowerPoint 미리보기는 흰 배경으로 그려집니다. 배경색을 핸들러에 전달하지만(`IPreviewHandlerVisuals`) Office 핸들러가 이를 무시합니다.
- 미리보기 창은 포커스를 가져가지 않으므로, 창 안의 스크롤은 마우스 휠로 합니다.
- 코드 서명이 없어 SmartScreen 경고가 뜹니다(위 설치 항목 참고).

## 라이선스

[GNU General Public License v3.0](LICENSE) (GPL-3.0-only)

이 프로젝트는 [QuickLook](https://github.com/QL-Win/QuickLook)(Copyright © QL-Win contributors, GPL-3.0)의
구현을 참고하거나 일부 가져와 수정했습니다. 해당 파일 머리말에 출처와 변경 사실을 표기합니다.

v0.1 골격 커밋(18fe705)까지는 MIT 로 공개됐으며, 이후 버전부터 GPL-3.0 을 따릅니다.

### 서드파티 라이브러리

| 라이브러리 | 라이선스 |
|---|---|
| Electron | MIT |
| pdf.js (pdfjs-dist) | Apache-2.0 |
| SheetJS (xlsx) | Apache-2.0 |
| mammoth | BSD-2-Clause |
| JSZip | MIT (MIT 또는 GPL-3.0 이중 라이선스 중 MIT 선택) |
| highlight.js | BSD-3-Clause |
| markdown-it | MIT |
| github-markdown-css | MIT |
| DOMPurify | Apache-2.0 / MPL-2.0 |
| CsWin32 (빌드 도구) | MIT |
