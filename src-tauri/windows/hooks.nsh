; Peekaboo NSIS 설치기 훅. `tauri.conf.json` 의 `bundle.windows.nsis.installerHooks` 가 이 파일을 가리키고,
; tauri-bundler 의 기본 템플릿(installer.nsi)이 `!ifmacrodef NSIS_HOOK_<이름>` 으로 아래 매크로를 부른다.
; 변수·매크로 이름은 @tauri-apps/cli 2.12.4 가 만든 installer.nsi 에서 확인한 것이다.
; 이 파일은 템플릿의 `Var UpdateMode` 선언보다 먼저 포함되므로(installer.nsi 의 !include 위치), 함수 안에서는 $UpdateMode 를 쓰지 못한다.
; $UpdateMode 검사는 매크로(섹션 안으로 펼쳐진다)에서만 한다.
;
; 구성(훅 하나당 매크로 하나)
;   NSIS_HOOK_PREINSTALL     옛 Electron 설치본(HKCU)을 조용히 제거하고, 시작 시 실행 값을 옛 이름에서 새 이름으로 옮길 준비를 한다.
;                            Section Install 맨 앞(바로가기를 만들기 전)이라, 옛 제거 프로그램이 지우는 같은 이름의 바로가기와 부딪히지 않는다.
;   NSIS_HOOK_POSTINSTALL    옛 설치본이 시작 시 실행에 등록돼 있었으면 새 exe 로 다시 쓴다.
;   NSIS_HOOK_POSTUNINSTALL  제거 뒤 정리: 시작 시 실행 값(Run·StartupApproved\Run)을 지운다.
; 앱 안 업데이트(`/UPDATE`, $UpdateMode = 1)로 실행됐을 때는 옛 Electron 처리를 건너뛴다.
;
; 값 이름은 src-tauri/src/startup.rs 의 VALUE_NAME(앱 identifier)과 같아야 한다.
; 기본 템플릿도 Run 값을 지우지만 이름이 PRODUCTNAME("Peekaboo")이라 앱이 쓰는 값과 다르다.
;
; 옛 제거 프로그램(electron-builder 26.18.0 의 템플릿: include/installUtil.nsh 의 uninstallOldVersion)을 따라 부른다.
;   - 제거 프로그램을 $PLUGINSDIR 로 복사해 실행한다. 설치 폴더의 파일 그대로 실행하면 제거 프로그램이 자기 파일이 든 폴더를 지우지 못한다.
;     복사본을 실행하지 못하면(정책 등으로 막힘, ExecWait 오류) 설치 폴더의 파일을 제자리에서 한 번 실행한다(electron-builder 의 TryInPlace).
;   - `_?=<설치 폴더>` 를 마지막 인자로 준다. 없으면 NSIS 제거 프로그램이 임시 폴더로 자신을 복사해 다시 실행하고 즉시 끝나
;     ExecWait 가 기다리지 못한다. 경로는 따옴표로 감싸지 않는다(NSIS 규약).
;   - `/S` 조용히, `/currentuser` 현재 사용자 설치, `--updated` 는 사용자 데이터(%APPDATA%\Peekaboo) 삭제를 막는다
;     (uninstaller.nsh: `--delete-app-data` 가 없고 isUpdated 이면 isDeleteAppData 가 0). `/KEEP_APP_DATA` 는 electron-builder 가 함께 주는 인자다.
;   - 실행 중인 옛 Peekaboo.exe 는 제거 프로그램이 직접 끝낸다(조용한 모드에서 확인 없이).
;   - 종료 코드 0 만으로 성공이라 보지 않고, 제거 뒤 HKCU 의 옛 제거 키가 사라졌는지 다시 읽어 확인한다(uninstaller.nsh 가 끝에서 지운다).
;   - `_?=` 로 실행한 제거 프로그램은 자기 파일을 지우지 못하므로 끝난 뒤 이 훅이 제거 프로그램과 빈 옛 폴더를 정리한다
;     (템플릿의 재설치 경로도 같은 일을 한다: installer.nsi 의 `reinst_uninstall`).
; 안전 장치(HKCU 의 UninstallString 은 사용자 권한으로 쓸 수 있는 값이라 신뢰하지 않는다)
;   - 설치기가 승격돼 있으면(관리자로 실행) 옛 제거를 건너뛴다. 승격된 프로세스가 사용자가 바꿀 수 있는 레지스트리 값의 경로를 실행하게 되기 때문이다.
;     승격 여부는 토큰의 TokenElevation 으로 본다(UserInfo::GetAccountType 은 "계정 종류"를 묻는 것이라 승격 여부와 같다고 확신할 수 없어 쓰지 않았다).
;     확인에 실패하면 승격된 것으로 보고 건너뛴다. 비승격 실행에서 0 이 나오는 것은 임시 NSIS 설치기로 확인했다(승격 실행은 UAC 가 필요해 확인하지 못했다).
;   - 경로는 `X:\` 로 시작하는 드라이브 절대 경로여야 하고(UNC `\\호스트\...`·`//호스트/...`·장치 경로 거부), 파일 이름이
;     `Uninstall Peekaboo.exe`(electron-builder 의 UNINSTALL_FILENAME)여야 한다. 아니면 실행하지 않는다.
; "모든 사용자"(HKLM)로 설치한 옛 버전은 관리자 권한이 필요해 건드리지 않고 설치 로그에만 남긴다(앱도 시작할 때 로그와 알림으로 안내한다: src-tauri/src/legacy.rs).
; 설치 로그 문구는 영어 ASCII 다. 이 파일은 BOM 없이 읽혀 한글이 깨질 수 있다.
;
; 시작 시 실행(Run) 값: 옛 버전은 값 이름이 버전마다 달랐을 수 있어(v0.1.0 등) 이름으로 찾지 않는다. HKCU Run 의 모든 값 중
; 데이터가 옛 설치 폴더의 Peekaboo.exe 를 가리키는 것을 찾아 새 이름(PEEKABOO_RUN_VALUE)으로 옮긴다. 작업 관리자의 "사용 안 함" 상태
; (StartupApproved\Run 의 같은 이름 값, REG_BINARY)도 새 이름으로 복사한다 — 값 이름이 바뀌면 상태가 이어지지 않기 때문이다.
;
; 재설치("먼저 제거" 선택): 템플릿이 설치 폴더의 uninstall.exe 를 `_?=<설치 폴더>` 로 직접 실행한다(installer.nsi 의 PageLeaveReinstall).
; 이때 이 제거 프로그램의 POSTUNINSTALL 이 Run 값을 지우면 같은 설치기가 이어서 설치해도 시작 시 실행이 사라진다.
; 제거 프로그램이 `_?=` 로 직접 실행됐는지는 인자로는 알 수 없지만(`_?=` 는 $CMDLINE 에서 빠진다) $EXEDIR 로 안다:
; 직접 실행이면 $EXEDIR 가 설치 폴더($INSTDIR)이고, 설정 > 앱 등에서 실행하면 NSIS 가 임시 폴더(~nsu*.tmp)로 복사해 실행하므로 다르다(실측).
; 앱 안 업데이트(`/UPDATE`)는 값을 그대로 둔다.

!define /ifndef PEEKABOO_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define /ifndef PEEKABOO_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!define /ifndef PEEKABOO_RUN_VALUE "io.github.kang-jacob-gitlb.peekaboo"
; electron-builder 가 appId(io.github.kang-jacob-gitlb.peekaboo)로 만든 GUID(UUID v5): 제거 키 이름이자 InstallLocation 을 둔 키 이름
!define /ifndef PEEKABOO_OLD_UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\a32366e0-3e8a-5400-a683-97d78e557198"
!define /ifndef PEEKABOO_OLD_INSTALL_KEY "Software\a32366e0-3e8a-5400-a683-97d78e557198"
!define /ifndef PEEKABOO_OLD_UNINSTALLER_NAME "Uninstall Peekaboo.exe"
!define /ifndef PEEKABOO_OLD_EXE_NAME "Peekaboo.exe"

; 옛 시작 시 실행 값이 있었으면 "1"(POSTINSTALL 이 새 exe 로 다시 쓴다)
Var PeekabooHadRun
; 옛 Electron 설치 폴더(끝의 \ 없음)
Var PeekabooOldDir
; 옛 제거가 끝나고 옛 제거 키가 사라졌으면 "1"
Var PeekabooRemoved
; 함수 사이에 값을 넘기는 임시 변수(스택 대신)
Var PeekabooArgA
Var PeekabooArgB
Var PeekabooArgC

; 설치기가 승격(관리자 권한 토큰)돼 실행 중이면 스택에 "1", 아니면 "0". 확인하지 못하면 "1"(안전한 쪽).
; OpenProcessToken(GetCurrentProcess 의 가짜 핸들 -1, TOKEN_QUERY=8) → GetTokenInformation(TokenElevation=20, TOKEN_ELEVATION{DWORD TokenIsElevated}).
Function PeekabooIsElevated
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  StrCpy $4 "1"
  System::Call 'advapi32::OpenProcessToken(p -1, i 8, *p .r1) i .r0'
  ${If} $0 <> 0
    System::Alloc 4
    Pop $2
    System::Call 'advapi32::GetTokenInformation(p r1, i 20, p r2, i 4, *i .r3) i .r0'
    ${If} $0 <> 0
      System::Call '*$2(i .r4)'
    ${EndIf}
    System::Free $2
    System::Call 'kernel32::CloseHandle(p r1)'
  ${EndIf}
  ${If} $4 <> 0
    StrCpy $4 "1"
  ${EndIf}
  StrCpy $0 $4
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Exch $0
FunctionEnd

; StartupApproved\Run 의 값(REG_BINARY)을 $PeekabooArgA 이름에서 $PeekabooArgB 이름으로 복사한다. 옛 값이 없으면 아무것도 하지 않는다.
; NSIS 내장 명령은 REG_BINARY 를 읽지 못해 advapi32 를 직접 부른다: RegOpenKeyExW → RegQueryValueExW(크기, 값) → RegSetValueExW → RegCloseKey.
Function PeekabooCopyApproved
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  ${If} $PeekabooArgA == $PeekabooArgB
    Goto peekaboo_ca_done
  ${EndIf}
  ; 0x80000001 = HKEY_CURRENT_USER, 0x2001f = KEY_READ | KEY_WRITE
  System::Call 'advapi32::RegOpenKeyExW(p 0x80000001, w "${PEEKABOO_APPROVED_KEY}", i 0, i 0x2001f, *p .r1) i .r0'
  ${If} $0 <> 0
    Goto peekaboo_ca_done
  ${EndIf}
  ; 데이터 버퍼를 NULL 로 주면 필요한 크기($3)와 종류($2)만 돌려준다
  System::Call 'advapi32::RegQueryValueExW(p r1, w "$PeekabooArgA", p 0, *i .r2, p 0, *i .r3) i .r0'
  ${If} $0 = 0
  ${AndIf} $3 > 0
    System::Alloc $3
    Pop $4
    System::Call 'advapi32::RegQueryValueExW(p r1, w "$PeekabooArgA", p 0, *i .r2, p r4, *i r3) i .r0'
    ${If} $0 = 0
      System::Call 'advapi32::RegSetValueExW(p r1, w "$PeekabooArgB", i 0, i r2, p r4, i r3) i .r0'
      ${If} $0 = 0
        DetailPrint "Copied the startup approval state to the new value name."
      ${EndIf}
    ${EndIf}
    System::Free $4
  ${EndIf}
  System::Call 'advapi32::RegCloseKey(p r1)'
  peekaboo_ca_done:
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; HKCU Run 의 모든 값 중 데이터가 `$PeekabooOldDir\Peekaboo.exe` 를 가리키는 것(따옴표·뒤따르는 인자는 허용)을 찾는다.
; 찾으면 $PeekabooHadRun = "1" 로 하고 StartupApproved 의 같은 이름 값을 새 이름(PEEKABOO_RUN_VALUE)으로 복사한다.
; $PeekabooArgC = "1" 이면 옛 Run 값을 지우고(새 이름과 다른 이름의 StartupApproved 값도) 다음 값을 본다. 지우면 뒤 값이 앞으로 당겨지므로 색인을 올리지 않는다.
; 삭제가 실패하면 같은 값을 다시 보지 않도록 색인을 올린다.
Function PeekabooMigrateRun
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7
  ${If} $PeekabooOldDir == ""
    Goto peekaboo_mr_done
  ${EndIf}
  StrCpy $0 "$PeekabooOldDir\${PEEKABOO_OLD_EXE_NAME}"
  StrLen $1 $0
  StrCpy $2 0
  peekaboo_mr_loop:
    ${If} $2 > 1000
      Goto peekaboo_mr_done
    ${EndIf}
    ClearErrors
    EnumRegValue $3 HKCU "${PEEKABOO_RUN_KEY}" $2
    ${If} ${Errors}
      Goto peekaboo_mr_done
    ${EndIf}
    ClearErrors
    ReadRegStr $4 HKCU "${PEEKABOO_RUN_KEY}" "$3"
    StrCpy $5 $4 1
    ${If} $5 == '"'
      StrCpy $4 $4 "" 1
    ${EndIf}
    ; 앞 N 글자가 옛 exe 경로와 같고(대소문자 무시) 그 뒤가 끝·따옴표·공백이면 옛 값이다
    StrCpy $5 $4 $1
    StrCpy $6 $4 1 $1
    StrCpy $7 "0"
    ${If} $3 != ""
    ${AndIf} $5 == $0
      ${If} $6 == ""
      ${OrIf} $6 == '"'
      ${OrIf} $6 == " "
        StrCpy $7 "1"
      ${EndIf}
    ${EndIf}
    ${If} $7 == "1"
      StrCpy $PeekabooHadRun "1"
      DetailPrint "Found an old startup entry: $3"
      StrCpy $PeekabooArgA $3
      StrCpy $PeekabooArgB "${PEEKABOO_RUN_VALUE}"
      Call PeekabooCopyApproved
      ${If} $PeekabooArgC == "1"
        ${If} $3 != "${PEEKABOO_RUN_VALUE}"
          DeleteRegValue HKCU "${PEEKABOO_APPROVED_KEY}" "$3"
        ${EndIf}
        ClearErrors
        DeleteRegValue HKCU "${PEEKABOO_RUN_KEY}" "$3"
        ${IfNot} ${Errors}
          Goto peekaboo_mr_loop
        ${EndIf}
      ${EndIf}
    ${EndIf}
    IntOp $2 $2 + 1
    Goto peekaboo_mr_loop
  peekaboo_mr_done:
  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; 옛 Electron 설치본(현재 사용자)을 조용히 제거한다. 실패해도 설치는 계속한다(Tauri 설치를 막지 않는다).
; 쓰는 레지스터는 모두 Push/Pop 으로 보존한다.
Function PeekabooRemoveOldElectron
  Push $R0
  Push $R1
  Push $R2
  Push $R3
  Push $R4
  Push $R5
  Push $R6
  Push $R7

  Call PeekabooIsElevated
  Pop $R0
  ${If} $R0 != "0"
    DetailPrint "Installer is elevated (or elevation could not be checked); skipping the old Electron removal."
    Goto peekaboo_done
  ${EndIf}

  ; 모든 사용자 설치(HKLM)는 건드리지 않고 알리기만 한다. 설치기는 32비트라 64비트 레지스트리 뷰를 명시한다.
  ; 템플릿의 .onInit 이 부르는 SetContext(utils.nsh)가 x64 에서 이미 SetRegView 64 로 맞춰 둔다. 같은 값을 다시 지정할 뿐이므로
  ; 읽은 뒤에 되돌리지 않는다(되돌려야 할 원래 값이 32 가 아니다). 32비트 Windows 에서는 뷰를 바꾸지 않는다.
  ${If} ${RunningX64}
    SetRegView 64
  ${EndIf}
  ReadRegStr $R0 HKLM "${PEEKABOO_OLD_UNINSTALL_KEY}" "UninstallString"
  ${If} $R0 != ""
    DetailPrint "Old Electron Peekaboo (all users) found. Left untouched: remove it from Windows Settings > Apps as administrator."
  ${EndIf}

  ReadRegStr $R0 HKCU "${PEEKABOO_OLD_UNINSTALL_KEY}" "UninstallString"
  ${If} $R0 == ""
    DetailPrint "No old Electron Peekaboo (current user) found."
    Goto peekaboo_done
  ${EndIf}

  ; 옛 설치 폴더: InstallLocation 이 있으면 그것, 없으면 아래에서 제거 프로그램이 든 폴더. 제거가 끝나면 키가 지워지므로 먼저 읽는다.
  ReadRegStr $PeekabooOldDir HKCU "${PEEKABOO_OLD_INSTALL_KEY}" "InstallLocation"
  StrCpy $R1 $PeekabooOldDir 1 -1
  ${If} $R1 == "\"
    StrLen $R2 $PeekabooOldDir
    IntOp $R2 $R2 - 1
    StrCpy $PeekabooOldDir $PeekabooOldDir $R2
  ${EndIf}

  ; UninstallString 은 `"<경로>\Uninstall Peekaboo.exe" /currentuser` 모양이다. 따옴표 안의 경로만 쓴다.
  StrCpy $R1 $R0 1
  ${If} $R1 != '"'
    DetailPrint "Old uninstall string is not quoted; skipping removal."
    Goto peekaboo_done
  ${EndIf}
  StrCpy $R0 $R0 "" 1
  StrCpy $R2 0
  peekaboo_find_quote:
    StrCpy $R3 $R0 1 $R2
    ${If} $R3 == ""
      DetailPrint "Old uninstall string has no closing quote; skipping removal."
      Goto peekaboo_done
    ${EndIf}
    ${If} $R3 == '"'
      Goto peekaboo_quote_found
    ${EndIf}
    IntOp $R2 $R2 + 1
    Goto peekaboo_find_quote
  peekaboo_quote_found:
  StrCpy $R0 $R0 $R2

  ; 드라이브 절대 경로(`X:\...`)만 허용한다. UNC(`\\호스트\...`, `//호스트/...`)·장치 경로(`\\?\`, `\\.\`)는 두 번째 글자가 ':' 가 아니라 걸러진다.
  StrCpy $R2 $R0 1 1
  StrCpy $R3 $R0 1 2
  ${If} $R2 != ":"
  ${OrIf} $R3 != "\"
    DetailPrint "Old uninstaller path is not a local drive path; skipping removal."
    Goto peekaboo_done
  ${EndIf}

  ; 설치 폴더 = 제거 프로그램이 든 폴더(마지막 \ 앞), 파일 이름 = 마지막 \ 뒤
  StrLen $R2 $R0
  peekaboo_find_slash:
    IntOp $R2 $R2 - 1
    ${If} $R2 < 0
      DetailPrint "Old uninstaller path has no folder; skipping removal."
      Goto peekaboo_done
    ${EndIf}
    StrCpy $R3 $R0 1 $R2
    ${If} $R3 != "\"
      Goto peekaboo_find_slash
    ${EndIf}
  StrCpy $R4 $R0 $R2
  IntOp $R2 $R2 + 1
  StrCpy $R3 $R0 "" $R2
  ${If} $R3 != "${PEEKABOO_OLD_UNINSTALLER_NAME}"
    DetailPrint "Old uninstaller is not named ${PEEKABOO_OLD_UNINSTALLER_NAME}; skipping removal."
    Goto peekaboo_done
  ${EndIf}
  ${If} $PeekabooOldDir == ""
    StrCpy $PeekabooOldDir $R4
  ${EndIf}

  ${IfNot} ${FileExists} "$R0"
    DetailPrint "Old uninstaller is gone; skipping removal."
    Goto peekaboo_done
  ${EndIf}

  InitPluginsDir
  StrCpy $R5 "$PLUGINSDIR\peekaboo-old-uninstall.exe"
  ClearErrors
  CopyFiles /SILENT "$R0" "$R5"
  ${If} ${Errors}
    ; 복사하지 못하면 제자리에서 실행한다(electron-builder 도 같은 대체 경로를 쓴다)
    StrCpy $R5 "$R0"
  ${EndIf}

  ; 실행 중인 앱이 끝나는 중이거나 파일이 잠겨 있을 수 있어 몇 번 다시 시도한다
  StrCpy $R6 0
  peekaboo_try:
    IntOp $R6 $R6 + 1
    ClearErrors
    DetailPrint "Removing old Electron Peekaboo (attempt $R6 of 3)..."
    ExecWait '"$R5" /S /KEEP_APP_DATA /currentuser --updated _?=$R4' $R7
    ${If} ${Errors}
      ; 복사본을 실행하지 못했다(정책 등). 설치 폴더의 제거 프로그램을 제자리에서 실행해 본다(electron-builder 의 TryInPlace).
      DetailPrint "Could not launch the uninstaller copy; trying it in place."
      ClearErrors
      ExecWait '"$R0" /S /KEEP_APP_DATA /currentuser --updated _?=$R4' $R7
    ${EndIf}
    ${IfNot} ${Errors}
    ${AndIf} $R7 = 0
      ; 종료 코드 0 이어도 제거 키가 남아 있으면 끝난 것이 아니다
      ClearErrors
      ReadRegStr $R1 HKCU "${PEEKABOO_OLD_UNINSTALL_KEY}" "UninstallString"
      ${If} $R1 == ""
        Goto peekaboo_removed
      ${EndIf}
      DetailPrint "Old uninstaller exited with 0 but its uninstall key is still there."
    ${Else}
      DetailPrint "Old uninstaller did not finish cleanly (exit code $R7)."
    ${EndIf}
    ${If} $R6 < 3
      Sleep 1000
      Goto peekaboo_try
    ${EndIf}
  DetailPrint "Could not remove the old Electron Peekaboo. Continuing; remove it manually from Windows Settings > Apps."
  StrCpy $PeekabooArgC "0"
  Goto peekaboo_migrate

  peekaboo_removed:
  DetailPrint "Old Electron Peekaboo removed."
  StrCpy $PeekabooRemoved "1"
  StrCpy $PeekabooArgC "1"
  ; `_?=` 로 실행한 제거 프로그램은 자기 파일을 지우지 못한다. 검증한 그 파일과 비어 있는 옛 폴더만 정리한다(하위 내용은 지우지 않는다).
  Delete "$R0"
  RMDir "$R4"

  peekaboo_migrate:
  Call PeekabooMigrateRun

  peekaboo_done:
  Pop $R7
  Pop $R6
  Pop $R5
  Pop $R4
  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
FunctionEnd

; 설치 시작(Section Install 맨 앞). 앱 안 업데이트에서는 옛 Electron 처리를 하지 않는다.
!macro NSIS_HOOK_PREINSTALL
  StrCpy $PeekabooHadRun "0"
  StrCpy $PeekabooRemoved "0"
  StrCpy $PeekabooOldDir ""
  ${If} $UpdateMode <> 1
    Call PeekabooRemoveOldElectron
  ${EndIf}
!macroend

; 설치 끝. 옛 설치본이 시작 시 실행에 등록돼 있었으면 새 이름으로 새 exe 를 등록한다(이미 있으면 덮는다).
; 작업 관리자의 "사용 안 함" 상태(StartupApproved)는 옛 이름에서 새 이름으로 이미 복사해 뒀다.
!macro NSIS_HOOK_POSTINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $PeekabooHadRun == "1"
    WriteRegStr HKCU "${PEEKABOO_RUN_KEY}" "${PEEKABOO_RUN_VALUE}" '"$INSTDIR\${MAINBINARYNAME}.exe"'
    DetailPrint "Re-registered Peekaboo to start with Windows."
  ${EndIf}
!macroend

; 제거가 끝난 뒤. 값을 지우지 않는 경우
;   - 업데이트(`/UPDATE`, $UpdateMode = 1)로 실행된 제거 단계
;   - 새 설치기가 재설치를 위해 `_?=<설치 폴더>` 로 직접 실행한 제거 프로그램($EXEDIR = $INSTDIR). 이어지는 설치가 값을 그대로 쓴다.
;     (사용자가 "먼저 제거" 뒤 설치를 취소하면 값이 남지만, 가리키는 exe 가 없을 뿐 다음 설치가 다시 쓴다.)
; 그 밖(설정 > 앱의 제거 등)에서는 지운다. 지우지 않으면 제거 뒤에도 로그인할 때 없는 exe 를 실행하려는 항목이 남는다.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $EXEDIR != $INSTDIR
    DeleteRegValue HKCU "${PEEKABOO_RUN_KEY}" "${PEEKABOO_RUN_VALUE}"
    DeleteRegValue HKCU "${PEEKABOO_APPROVED_KEY}" "${PEEKABOO_RUN_VALUE}"
  ${EndIf}
!macroend
