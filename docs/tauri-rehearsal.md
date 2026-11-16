# Tauri 설치기·업데이트 리허설

Tauri 설치기가 옛 Electron 설치본을 지우는 훅(`src-tauri/windows/hooks.nsh`)과 Tauri 자동 업데이트를 **Windows Sandbox 안에서** 시험하는 절차다
(계획: [tauri-migration.md](tauri-migration.md) 6절, M5). 도구는 `scripts/rehearsal/` 에 있다.

> **이 PC 에서 설치본을 그냥 실행하지 마라.** 설치기 훅은 사용자 PC 의 Electron Peekaboo 를 제거하는 코드이고, 리허설 스크립트는 Electron 을 설치·제거한다.
> 스크립트는 샌드박스의 기본 사용자(`WDAGUtilityAccount`)가 아니면 아무것도 하지 않고 끝난다. 호스트 쪽 `prepare.ps1` 은 폴더와 `.wsb` 만 만들고 설치본을 실행하지 않는다.

## 준비

- Windows 11 Pro/Enterprise 에서 "Windows Sandbox" 기능을 켠다(관리자 PowerShell: `Enable-WindowsOptionalFeature -Online -FeatureName Containers-DisposableClientVM`, 재부팅).
- 샌드박스는 닫으면 모든 상태가 사라진다. 호스트와는 매핑 폴더로만 오간다: `C:\Rehearsal\in`(설치본)과 `C:\Rehearsal\scripts`(리허설 스크립트)는 **읽기 전용**,
  `C:\Rehearsal\out`(결과)만 쓸 수 있다. 샌드박스 안의 설치본이나 옛 앱이 호스트의 입력·스크립트를 바꿀 수 없고, `.wsb` 는 매핑 폴더 밖(작업 폴더 바로 아래)에 만든다. 클립보드·프린터·오디오는 끈다.
- `prepare.ps1 -Work` 는 **비어 있거나 이 스크립트가 만든 폴더**(표식 파일 `.peekaboo-rehearsal`)여야 한다. `in`·`out`·`scripts` 를 비우므로, 표식이 없는 비어 있지 않은 폴더·드라이브 루트·
  저장소를 품은 폴더는 거부한다.
- 기본은 **네트워크를 끈다**: 옛 Electron 앱이 스스로 업데이트를 받으러 가지 않게 한다. Tauri 설치본은 WebView2 부트스트래퍼를 포함하지만(`embedBootstrapper`),
  샌드박스에 WebView2 런타임이 없어 내려받아야 하면 `prepare.ps1 -EnableNetwork` 로 켠다.

## 1. 옛 Electron → Tauri 이전 리허설

사용자가 직접 내려받아 둔 설치본이 필요하다.

- Electron: GitHub 릴리스의 `Peekaboo-Setup-<버전>-x64.exe`(예: v0.10.1). 계획서는 옛 버전 3개(0.2.0, 중간 하나, 최신)로 반복하라고 한다.
  `gh release download v0.10.1 -R Kang-Jacob-GitLB/peekaboo -p "Peekaboo-Setup-0.10.1-x64.exe" -D <폴더>`
- Tauri: `tauri-build` 워크플로 아티팩트(`peekaboo-tauri-installer`) 또는 로컬에서 `npm run dist:tauri` 로 만든 `release-tauri\Peekaboo-Setup-<버전>-x64.exe`.

```powershell
# 호스트(이 PC)에서 — 폴더와 .wsb 만 만든다
.\scripts\rehearsal\prepare.ps1 -Work D:\rehearsal -Scenario Migrate `
  -ElectronInstaller D:\dl\Peekaboo-Setup-0.10.1-x64.exe -TauriInstaller D:\dl\Peekaboo-Setup-X-x64.exe
# 출력된 명령으로 샌드박스를 띄운다 (로그온 때 sandbox-migrate.ps1 이 자동 실행된다)
WindowsSandbox.exe D:\rehearsal\rehearsal-migrate.wsb
```

샌드박스 안에서 스크립트가 하는 일: (a) Electron 설치(`/S /currentuser`) → 시작 시 실행(Run) 값 쓰기 → `%APPDATA%\Peekaboo\settings.json` 에 표식 쓰기 → 옛 앱 실행(실행 중인 옛 앱이 있어도 설치가 끝나는지 보려는 것),
(b) Tauri 설치본 무인 실행(`/S`, 제한 시간 240초), (c) 검증. 결과는 호스트의 `D:\rehearsal\out\migrate-result.json`·`migrate-result.txt`(진행 로그 `migrate-log.txt`).

| 검증 | 기대 |
|---|---|
| Tauri 설치기가 시간 안에 종료 코드 0 | 멈추거나 시간 초과하지 않음(계획서 6절 4번) |
| 옛 제거 키(HKCU, GUID `a32366e0-…`) 없음 | 제거됨 |
| `%LOCALAPPDATA%\Programs\Peekaboo` 없음 | 제거됨 |
| 새 설치 폴더(`%LOCALAPPDATA%\Peekaboo\Peekaboo.exe`)와 제거 키(`InstallLocation`) | 있음 |
| Run 값 `io.github.kang-jacob-gitlb.peekaboo` | `"<새 설치 폴더>\Peekaboo.exe"` |
| `%APPDATA%\Peekaboo\settings.json` | 표식이 남아 있음(설정 유지) |
| 바로가기 | 시작 메뉴에 정확히 1개, 옛 exe 를 가리키는 것 없음 |
| 프로세스 | 옛 폴더에서 실행 중인 `Peekaboo.exe`·`peekaboo-helper.exe` 없음 |
| (설치 직전) 옛 Electron 실행 중 | 설치기를 돌리기 직전에 옛 폴더에서 `Peekaboo.exe` 가 실제로 떠 있음(아니면 "실행 중이어도 설치가 끝난다"는 검증이 무의미해 FAIL) |
| 작업 관리자 "사용 안 함"(`-DisabledInTaskManager`) | `StartupApproved\Run` 의 새 이름 값이 옛 상태(12바이트)와 같고, 옛 이름이면 옛 이름 값은 없음 |
| Run 값이 원래 없음(`-RunValue None`) | 설치 뒤에도 Run 값을 새로 만들지 않음 |
| v0.1.0 계열(`-RunValue LegacyName`, 옛 이름 `electron.app.Peekaboo` 가정) | 옛 이름 Run 값은 사라지고 새 이름 값이 새 exe 를 가리킴 |

시나리오는 `prepare.ps1` 옵션으로 고른다(`-RunValue Current|LegacyName|None`, `-LegacyRunName <이름>`, `-DisabledInTaskManager`). 위 세 가지(옛 이름·값 없음·사용 안 함)를 각각 한 번씩 돌린다.
v0.1.0 은 `app.setLoginItemSettings({ openAtLogin })` 만 부르고 `name` 을 주지 않으며 `setAppUserModelId` 도 없다. Electron 문서상 값 이름의 기본은 AppUserModelId 이고 Electron 의 기본 AppUserModelId 는 `electron.app.<앱 이름>` 이라고 알고 있어 `electron.app.Peekaboo` 를 기본으로 뒀다(**미확인 가정**). 실제 v0.1.0 설치본을 샌드박스에서 설치해 로그인 항목을 켜 보고 `HKCU\...\Run` 의 값 이름을 확인한 뒤 `-LegacyRunName` 에 넣는다. 훅은 이름이 아니라 경로로 찾으므로 이름이 달라도 동작한다.

실패하면 `migrate-result.json` 의 `steps`(설치 종료 코드, 설치 전후 Run 값·바로가기·프로세스)를 본다. 설치기의 `DetailPrint` 는 일반 설치(비조용) 모드에서만 보이므로,
훅 동작을 눈으로 보려면 샌드박스 안에서 설치본을 `/S` 없이 직접 실행해 본다.

**화면이 있는(비조용) 설치는 수동 절차다**(자동화하지 않는다). 샌드박스를 새로 띄워 스크립트가 자동 실행되지 않게 하고(`.wsb` 의 `LogonCommand` 를 지운 사본을 쓴다) 사람이 한다.
1. Electron 설치본을 설치하고 앱을 띄운다. 2. Tauri 설치본을 더블클릭해 마법사로 설치한다(UAC 없이 현재 사용자). 3. 설치 로그(`DetailPrint`)에서 옛 제거·Run 값 이동 문구를 확인한다.
4. 같은 Tauri 설치본을 한 번 더 실행해 재설치 페이지에서 "먼저 제거"를 고르고 설치를 끝낸 뒤 Run 값이 남아 있는지 본다(재설치 경로는 시작 시 실행을 지우지 않는다 — hooks.nsh 의 `$EXEDIR = $INSTDIR` 판별).
   설정 > 앱에서 제거하면 Run 값이 사라져야 한다.

## 2. Tauri → Tauri 업데이트 리허설(로컬 서버)

실제 GitHub 릴리스에 요청을 보내지 않는다. 엔드포인트를 `http://127.0.0.1:8123/latest.json` 으로 박은 시험 빌드 두 개(예: 0.1.0, 0.2.0)와 **시험용 키**가 필요하다.
버전 `0.0.0` 은 업데이트가 꺼지므로 `npm pkg set version=…` 으로 정한다. 이 변경은 시험용이니 **임시 체크아웃에서만** 하고 커밋하지 않는다(저장소의 `0.0.0` 유지).

공개키는 환경 변수가 아니라 `src-tauri/updater.pub` 파일에서만 읽는다. 임시 체크아웃에 시험 공개키를 **그 파일로** 복사한다(커밋 금지).
`scripts/tauri-build.mjs` 는 `CI` 환경 변수가 있으면 기본(GitHub) 외 엔드포인트를 거부하므로 이 시험 빌드는 로컬에서만 한다.

```powershell
# 1) 시험용 키 한 번 만들기 (비밀번호는 비움). 개인키는 저장소·로그에 두지 않는다.
npx tauri signer generate --ci -p "" -w D:\rehearsal-key\test.key
# 2) 임시 체크아웃에서 두 버전을 차례로 빌드. 시험 키 환경 변수는 자식 셸 안에서만 쓰고, 셸이 끝나면 함께 사라진다(이 셸에 남기지 않는다).
Copy-Item D:\rehearsal-key\test.key.pub src-tauri\updater.pub
powershell -NoProfile -Command {
  $env:TAURI_SIGNING_PRIVATE_KEY = 'D:\rehearsal-key\test.key'
  $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
  $env:PEEKABOO_UPDATER_ENDPOINT = 'http://127.0.0.1:8123/latest.json'
  foreach ($v in '0.1.0', '0.2.0') {
    npm pkg set version=$v
    npm run dist:tauri
    Copy-Item "release-tauri\Peekaboo-Setup-$v-x64.exe*" D:\update-feed\
  }
}
# 3) 새 버전(0.2.0)의 latest.json — 설치본 주소를 로컬 서버로. 서명을 updater.pub 로 검증한 뒤에만 만든다.
node scripts/update-feeds.mjs --version 0.2.0 --base-url http://127.0.0.1:8123 --dir D:\update-feed --out D:\update-feed\latest.json
# 4) 호스트: 폴더·.wsb 준비 (v1 설치본, v2 설치본, latest.json 이 UpdateDir 에 있어야 한다)
.\scripts\rehearsal\prepare.ps1 -Work D:\rehearsal -Scenario Update -UpdateDir D:\update-feed -OldVersion 0.1.0 -NewVersion 0.2.0
WindowsSandbox.exe D:\rehearsal\rehearsal-update.wsb
```

샌드박스 안에서: 0.1.0 설치 → Run 값·설정 파일 표식 만들기 → `serve-feed.ps1`(127.0.0.1:8123, 숨김 창) 시작 → 앱 실행 → 앱 로그(`%APPDATA%\Peekaboo\logs\peekaboo.log`)에
`downloaded 0.2.0 …, signature verified` 가 나올 때까지 최대 120초 대기 → `out\update-install-result.json`.
그다음 **사람이** 샌드박스 창에서 트레이 아이콘 우클릭 → "다시 시작하여 업데이트 (0.2.0)" 를 누르고(진행 표시줄이 보이는 passive 설치), 설치가 끝나면 샌드박스 PowerShell 에서:

```powershell
C:\Rehearsal\scripts\sandbox-update.ps1 -Phase Verify
```

`out\update-verify-result.json`: 설치된 버전(`DisplayVersion`) 0.2.0, Run 값 유지, 앱 재실행(`/R`), 헬퍼 프로세스 정확히 1개, 설정 파일 유지. 트레이 "종료" 경로(재실행하지 않고 설치)도
같은 방법으로 따로 확인할 수 있다(1번 단계 뒤 "종료"를 누르고 Verify 대신 직접 `DisplayVersion` 과 프로세스 부재 확인).

## 알려진 한계

- 이 리허설 도구는 만들기만 했고 **아직 실행해 보지 않았다**(Windows PowerShell 5.1·PowerShell 7 의 문법 검사만 했다). 첫 실행에서 고칠 점이 나올 수 있다.
  설치기 훅(`hooks.nsh`)의 로직은 샌드박스 없이 따로 시험했다: 같은 파일을 시험 키(`HKCU\Software\PeekabooTest-…`)만 가리키게 해 임시 NSIS 설치기에 포함시켜,
  옛 이름 Run 값 이동·StartupApproved 복사·제거 실패 판정·경로 거부·재설치/독립 제거 구분을 확인했다. 승격된 설치기에서의 건너뛰기와 실제 electron-builder 제거 프로그램은 확인하지 못했다.
- 샌드박스에서는 Electron 앱의 키보드 훅·탐색기 동작까지는 보지 않는다. 설치·제거·등록 상태만 본다.
- 코드 서명이 없어 SmartScreen 경고가 뜰 수 있다(샌드박스에서는 조용한 설치라 보이지 않는다).
