; Peekaboo NSIS 설치기 훅. `tauri.conf.json` 의 `bundle.windows.nsis.installerHooks` 가 이 파일을 가리키고,
; tauri-bundler 의 기본 템플릿(installer.nsi)이 `!ifmacrodef NSIS_HOOK_<이름>` 으로 아래 매크로를 부른다.
; 변수·매크로 이름은 @tauri-apps/cli 2.12.4 가 만든 installer.nsi 에서 확인한 것이다.
;
; 구성(훅 하나당 매크로 하나)
;   NSIS_HOOK_POSTUNINSTALL  제거 뒤 정리: 시작 시 실행 값(Run·StartupApproved\Run)을 지운다.
;   NSIS_HOOK_PREINSTALL     (M5 에서 추가 예정) 옛 Electron 설치본 제거 — docs/tauri-migration.md 6절.
;   NSIS_HOOK_POSTINSTALL    (M5 에서 추가 예정) 옛 설치본의 Run 값 재등록.
;
; 값 이름은 src-tauri/src/startup.rs 의 VALUE_NAME(앱 identifier)과 같아야 한다.
; 기본 템플릿도 Run 값을 지우지만 이름이 PRODUCTNAME("Peekaboo")이라 앱이 쓰는 값과 다르다.

!define PEEKABOO_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define PEEKABOO_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!define PEEKABOO_RUN_VALUE "io.github.kang-jacob-gitlb.peekaboo"

; 제거가 끝난 뒤. 업데이트(`/UPDATE`, $UpdateMode = 1)로 실행된 제거 단계는 값을 그대로 둔다.
; 지우지 않으면 제거 뒤에도 로그인할 때 없는 exe 를 실행하려는 항목이 남는다.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "${PEEKABOO_RUN_KEY}" "${PEEKABOO_RUN_VALUE}"
    DeleteRegValue HKCU "${PEEKABOO_APPROVED_KEY}" "${PEEKABOO_RUN_VALUE}"
  ${EndIf}
!macroend
