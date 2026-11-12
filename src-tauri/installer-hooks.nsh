; Reelink is a small desktop utility, so its entry point belongs on the desktop.
!macro NSIS_HOOK_POSTINSTALL
  ; A versioned, standalone ICO avoids stale EXE icon entries in Explorer.
  CreateShortcut "$DESKTOP\Reelink.lnk" "$INSTDIR\reelink.exe" "" "$INSTDIR\icons\reelink-${VERSION}.ico" 0
  !insertmacro SetLnkAppUserModelId "$DESKTOP\Reelink.lnk"
  ; The headless entry strictly loads saved preferences before creating the
  ; capture link and refuses to replace a same-named link owned by another target.
  ClearErrors
  ExecWait '"$INSTDIR\reelink.exe" --install-capture-shortcut' $0
  ${If} ${Errors}
    DetailPrint "Reelink capture shortcut setup could not be started."
  ${ElseIf} $0 != 0
    DetailPrint "截图启动快捷键配置失败。请打开 Reelink 检查快捷键；如有恢复备份会保留。"
  ${EndIf}
  ; Tauri's finish-page callback preserves the icon when it recognizes this target.
  ; Old-binary cleanup and Start menu migration have already completed at this point.
  StrCpy $OldMainBinaryName "${MAINBINARYNAME}.exe"
  ; Refresh only our shortcut, with a Unicode path; do not delete global caches.
  System::Call 'shell32::SHChangeNotify(i 0x2000, i 0x1005, w "$DESKTOP\Reelink.lnk", p 0)'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$DESKTOP\Reelink.lnk"
  ; Run before the executable is removed. Only its exact --capture link is owned.
  ClearErrors
  ExecWait '"$INSTDIR\reelink.exe" --remove-capture-shortcut' $0
  ${If} ${Errors}
    DetailPrint "Reelink capture shortcut cleanup could not be started."
  ${ElseIf} $0 != 0
    DetailPrint "Reelink capture shortcut cleanup did not complete."
  ${EndIf}
!macroend
