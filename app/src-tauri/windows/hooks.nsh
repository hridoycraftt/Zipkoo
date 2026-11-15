; Zipkoo NSIS hooks: add the right-click menu after install, remove it before uninstall.
; The app does the registry work itself so the installer, the MSI and the in-app switch share one code path.
!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec '"$INSTDIR\Zipkoo.exe" --register-shell'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\Zipkoo.exe" --unregister-shell'
!macroend
