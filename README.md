# Peekaboo

Windows 탐색기에서 파일을 선택하고 **스페이스바**를 누르면 바로 미리보기 창이 뜨는 상주 프로그램입니다.
macOS Quick Look 과 비슷한 사용감을 목표로, 처음부터 새로 작성했습니다.

> 🚧 개발 중 (v0.1) — 아래 지원 포맷 중 체크된 것만 동작합니다.

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
트레이 아이콘 메뉴에서 단축키 켜기/끄기, Windows 시작 시 실행을 설정할 수 있습니다.

## 지원 포맷

- [ ] 이미지: jpg, png, gif, webp, svg
- [ ] 텍스트 / 소스 코드 (구문 강조)
- [ ] Markdown
- [ ] PDF
- [ ] 영상 / 오디오
- [ ] 폴더 (파일 목록)
- [ ] zip 목록
- [ ] docx / xlsx 간단 미리보기

## 설치

[Releases](../../releases) 에서 `Peekaboo-Setup-x.y.z.exe` 를 받아 실행합니다.

> ⚠️ **SmartScreen 경고 안내** — 아직 코드 서명 인증서로 서명하지 않은 빌드라서, 처음 실행할 때
> "Windows의 PC 보호" 경고가 뜹니다. **추가 정보 → 실행** 을 누르면 설치됩니다.
> 불안하다면 아래 방법으로 소스에서 직접 빌드할 수 있습니다.

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

헬퍼만 단독으로 시험하려면:

```powershell
npm run helper:console
```

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
- 미리보기 창은 포커스를 가져가지 않으므로, 창 안의 스크롤은 마우스 휠로 합니다.
- 코드 서명이 없어 SmartScreen 경고가 뜹니다(위 설치 항목 참고).

## 라이선스

[MIT](LICENSE)
