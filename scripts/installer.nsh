; Zusatz zum Installer (electron-builder, nsis.include), ENTSCHEIDE E39. Nur ASCII: makensis liest die Datei ohne BOM.
; Das mitgelieferte Python entpackt die App selbst nach %LOCALAPPDATA%\ASK-Desktop\python (src/main/python.js).
; Beim Deinstallieren entfernen, nicht bei einem Update (dort braucht die neue Version es meist weiter).
; Einstellungen und Chats bleiben wie bisher.
!macro customUnInstall
  ${ifNot} ${isUpdated}
    RMDir /r "$LOCALAPPDATA\ASK-Desktop\python"
  ${endIf}
!macroend
