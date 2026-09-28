; Reelink is a small desktop utility, so its entry point belongs on the desktop.
!macro NSIS_HOOK_POSTINSTALL
  ; A versioned, standalone ICO avoids stale EXE icon entries in Explorer.
  CreateShortcut "$DESKTOP\Reelink.lnk" "$INSTDIR\reelink.exe" "" "$INSTDIR\icons\reelink-${VERSION}.ico" 0
  !insertmacro SetLnkAppUserModelId "$DESKTOP\Reelink.lnk"
  ; Tauri's finish-page callback preserves the icon when it recognizes this target.
  ; Old-binary cleanup and Start menu migration have already completed at this point.
  StrCpy $OldMainBinaryName "${MAINBINARYNAME}.exe"
  ; Refresh only our shortcut, with a Unicode path; do not delete global caches.
  System::Call 'shell32::SHChangeNotify(i 0x2000, i 0x1005, w "$DESKTOP\Reelink.lnk", p 0)'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$DESKTOP\Reelink.lnk"
!macroend
