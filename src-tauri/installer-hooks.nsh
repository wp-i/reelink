; Reelink is a small desktop utility, so its entry point belongs on the desktop.
!macro NSIS_HOOK_POSTINSTALL
  CreateShortcut "$DESKTOP\Reelink.lnk" "$INSTDIR\reelink.exe" "" "$INSTDIR\reelink.exe" 0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$DESKTOP\Reelink.lnk"
!macroend
