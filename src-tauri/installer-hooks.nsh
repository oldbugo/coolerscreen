; CoolerScreen installer hooks (Tauri NSIS, per-user install, no admin).
;
; 1. Replace a CoolerScreen 1.x install (the Electron build, normally %LOCALAPPDATA%\Programs\CoolerScreen):
;    its program files are removed (only when the Electron marker file is there, so an unrelated folder is never
;    touched), and the old Start Menu / Desktop shortcut is repointed in place to the new exe, so a pinned
;    shortcut keeps working. The user's data (%APPDATA%\CoolerScreen, Documents\CoolerScreen\Gallery) is never touched.
; 2. Install the recovery tool cs2.exe next to the app (see README: Troubleshooting).

!macro CS_REMOVE_LEGACY DIR
  IfFileExists "${DIR}\resources\app\electron\main.js" 0 +2
    RMDir /r "${DIR}"
!macroend

!macro CS_REPOINT_LNK LNK
  IfFileExists "${LNK}" 0 +2
    CreateShortcut "${LNK}" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\${MAINBINARYNAME}.exe" 0
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro CS_REMOVE_LEGACY "$LOCALAPPDATA\Programs\CoolerScreen"
  !insertmacro CS_REMOVE_LEGACY "$INSTDIR"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File "..\..\cs2.exe"
  !insertmacro CS_REPOINT_LNK "$SMPROGRAMS\CoolerScreen.lnk"
  !insertmacro CS_REPOINT_LNK "$DESKTOP\CoolerScreen.lnk"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\cs2.exe"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
