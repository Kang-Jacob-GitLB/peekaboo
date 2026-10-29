; electron-builder 가 buildResources(resources/)의 installer.nsh 를 자동으로 포함한다.
; 앱(Peekaboo.exe)이 떠 있는지는 electron-builder 가 확인하지만, 별도 프로세스인 헬퍼(peekaboo-helper.exe)는 모른다.
; 앱 안 업데이트는 앱 종료 직후 설치를 시작하므로, 헬퍼가 아직 내려가는 중이면 exe 를 덮어쓰지 못할 수 있다.
; 남아 있는 헬퍼만 끝낸다(앱이 계속 떠 있다면 앱이 헬퍼를 다시 띄운다).

!macro customInit
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM peekaboo-helper.exe'
  Pop $0
!macroend

!macro customUnInit
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM peekaboo-helper.exe'
  Pop $0
!macroend
