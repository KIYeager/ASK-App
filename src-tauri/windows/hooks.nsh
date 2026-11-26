; Ergänzungen zum NSIS-Installer von Tauri (bundle.windows.nsis.installerHooks), ENTSCHEIDE E39 und E57.
; Nur ASCII: makensis liest die Datei ohne BOM.

!macro NSIS_HOOK_PREINSTALL
  ; Uebergang von der Electron-Fassung (bis 0.3.x): deren Deinstallation still ausfuehren und warten.
  ; Mit --updated behaelt sie das entpackte Python; Einstellungen und Chats (%APPDATA%\ASK) bleiben ohnehin.
  ${If} ${FileExists} "$LOCALAPPDATA\Programs\ask-desktop\Uninstall ASK.exe"
    DetailPrint "Fruehere Fassung von ASK wird entfernt ..."
    ExecWait '"$LOCALAPPDATA\Programs\ask-desktop\Uninstall ASK.exe" /S --updated _?=$LOCALAPPDATA\Programs\ask-desktop'
    RMDir /r "$LOCALAPPDATA\Programs\ask-desktop"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; electron-updater startet den neuen Installer still mit --force-run und erwartet, dass er ASK danach startet
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "--force-run" $R1
  ${IfNot} ${Errors}
  ${AndIf} ${Silent}
    Exec '"$INSTDIR\ASK.exe"'
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Das mitgelieferte Python entpackt die App selbst nach %LOCALAPPDATA%\ASK-Desktop\python (python.rs).
  ; Beim Deinstallieren entfernen, nicht bei einem Update. Einstellungen und Chats bleiben.
  ${If} $UpdateMode <> 1
    RMDir /r "$LOCALAPPDATA\ASK-Desktop\python"
  ${EndIf}
!macroend
