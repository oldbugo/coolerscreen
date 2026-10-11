; CoolerScreen installer hooks (Tauri NSIS, per-user install, no admin).
;
; Replaces a CoolerScreen 1.x install (the Electron build, normally %LOCALAPPDATA%\Programs\CoolerScreen):
;   - its program files are removed, but only when the Electron marker file is there, so an unrelated folder is
;     never touched;
;   - the old Start Menu / Desktop "CoolerScreen.lnk" is repointed to the new exe (same name and place, so a
;     pinned shortcut keeps working).
; The user's data (%APPDATA%\CoolerScreen, Documents\CoolerScreen\Gallery) is never touched.
; (cs2.exe, the recovery tool, is installed by the bundler itself.)

!macro CS_REMOVE_LEGACY DIR TAG
  IfFileExists "${DIR}\resources\app\electron\main.js" 0 cs_done_${TAG}
    Push $0
    ClearErrors
    FileOpen $0 "${DIR}\CoolerScreen.exe" a   ; cannot be opened for writing while 1.x is still running
    IfErrors cs_locked_${TAG}
    FileClose $0
    Pop $0
    RMDir /r "${DIR}"
    Goto cs_done_${TAG}
  cs_locked_${TAG}:
    Pop $0
    MessageBox MB_OK|MB_ICONEXCLAMATION "CoolerScreen 1.x is still running. Close it, then run this installer again."
    Abort
  cs_done_${TAG}:
!macroend

!macro CS_REPOINT_LNK LNK
  IfFileExists "${LNK}" 0 +2
    CreateShortcut "${LNK}" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro CS_REMOVE_LEGACY "$LOCALAPPDATA\Programs\CoolerScreen" a
  !insertmacro CS_REMOVE_LEGACY "$INSTDIR" b
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro CS_REPOINT_LNK "$SMPROGRAMS\CoolerScreen.lnk"
  !insertmacro CS_REPOINT_LNK "$DESKTOP\CoolerScreen.lnk"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
