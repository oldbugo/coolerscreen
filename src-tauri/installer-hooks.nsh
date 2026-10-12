; CoolerScreen installer hooks (Tauri NSIS, per-user install, no admin).
;
; Replaces a CoolerScreen 1.x install (the Electron build, normally %LOCALAPPDATA%\Programs\CoolerScreen):
;   - its program files are removed, but only when the Electron marker file is there, so an unrelated folder is
;     never touched;
;   - the old Start Menu / Desktop "CoolerScreen.lnk" is repointed to the new exe (same name and place, so a
;     pinned shortcut keeps working).
; The user's data (%APPDATA%\CoolerScreen, Documents\CoolerScreen\Gallery) is never touched.
; (cs2.exe, the recovery tool, is installed by the bundler itself.)
;
; Apps & Features: 1.x was installed by scripts\install-local.ps1, which writes NO registry entry (checked on the
; development PC: no HKCU\...\Uninstall\CoolerScreen exists), so there is nothing stale to remove. Should a 1.x entry
; ever exist, it has the same key name as the one this installer writes (Uninstall\CoolerScreen, from the product
; name), so it is simply overwritten; installing 2.x twice also reuses that one key (no duplicates).
; Uninstall: the bundler's own uninstaller deletes the Start Menu and Desktop "CoolerScreen.lnk" only when they point at
; our exe (IsShortcutTarget), which is exactly the shortcut repointed here; a shortcut the user retargeted stays.

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
  ; The opt-in "Start CoolerScreen when Windows starts" entry (Settings > Live screen) must not outlive the app.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "CoolerScreen"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
