; Zusatz zum Installer (electron-builder, nsis.include), ENTSCHEIDE E39 und E59. Nur ASCII: makensis liest die Datei ohne BOM.
; Das mitgelieferte Python entpackt die App selbst nach %LOCALAPPDATA%\ASK-Desktop\python (src/main/python.js).
; Beim Deinstallieren entfernen, nicht bei einem Update (dort braucht die neue Version es meist weiter).
; Einstellungen und Chats bleiben wie bisher.
!macro customUnInstall
  ${ifNot} ${isUpdated}
    RMDir /r "$LOCALAPPDATA\ASK-Desktop\python"
  ${endIf}
!macroend

; Fortschrittsfenster (ENTSCHEIDE E59). electron-builder zeigt es mit dem Plugin SpiderBanner, gleich danach
; kommt diese Stelle (CHECK_APP_RUNNING in installSection.nsh). Hier gestalten wir das Fenster um und pruefen
; danach wie electron-builder, ob ASK noch laeuft. Steuerelemente des Plugins (Dialog 104, Masse in
; Dialog-Einheiten): 1000 Titel, 1002 zweite Zeile (bekommt sonst nichts), 1001 Balken, 1003 Statuszeile von
; NSIS (bleibt leer, SetDetailsPrint none), 1025 Symbol. Der Hintergrund des Fensters gehoert dem Plugin.
; Mit customCheckAppRunning bindet electron-builder getProcessInfo.nsh und Var pid nicht mehr ein
; (include/allowOnlyOneInstallerInstance.nsh); _CHECK_APP_RUNNING braucht beides, auch im Deinstaller.
!include "getProcessInfo.nsh"
Var pid

; Eigene Namen: electron-builder liest diese Datei vor WinMessages.nsh, das PBM_SETBARCOLOR ohne !ifndef
; definiert; ein gleichnamiges !define hier bricht den Build ab (Release 0.3.9).
!define ASK_PBM_SETBARCOLOR 0x0409
!define ASK_PBM_SETBKCOLOR 0x2001

; Steuerelement ID im Dialog HWND an x y b h (Dialog-Einheiten) setzen, umgerechnet wie das Plugin selbst
!macro askPlace HWND ID X Y W H
  GetDlgItem $3 ${HWND} ${ID}
  System::Call '*(i ${X}, i ${Y}, i ${W}, i ${H}) p .r4'
  System::Call 'user32::MapDialogRect(p ${HWND}, p r4)'
  System::Call '*$4(i .r5, i .r6, i .r7, i .r8)'
  System::Free $4
  ; SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED
  System::Call 'user32::SetWindowPos(p r3, p 0, i r5, i r6, i r7, i r8, i 0x34)'
!macroend

!macro askStyleBanner
  FindWindow $0 "#32770" "" $HWNDPARENT
  FindWindow $0 "#32770" "" $HWNDPARENT $0
  ${If} $0 <> 0
    SendMessage $HWNDPARENT ${WM_SETTEXT} 0 "STR:ASK"

    CreateFont $1 "Segoe UI" 11 600
    CreateFont $2 "Segoe UI" 9 400
    ${If} ${isUpdated}
      StrCpy $4 "ASK wird aktualisiert"
      StrCpy $5 "Chats und Einstellungen bleiben. ASK startet danach von selbst."
    ${Else}
      StrCpy $4 "ASK wird installiert"
      StrCpy $5 "Einen Moment bitte. ASK startet danach von selbst."
    ${EndIf}
    GetDlgItem $3 $0 1000
    SendMessage $3 ${WM_SETFONT} $1 0
    SendMessage $3 ${WM_SETTEXT} 0 "STR:$4"
    GetDlgItem $3 $0 1002
    SendMessage $3 ${WM_SETFONT} $2 0
    SendMessage $3 ${WM_SETTEXT} 0 "STR:$5"
    GetDlgItem $3 $0 1003
    ShowWindow $3 ${SW_HIDE}

    ; Balken flach in ASK-Farbe: ohne Windows-Design gelten die eigenen Farben, ohne Rahmen (WS_BORDER)
    GetDlgItem $3 $0 1001
    System::Call 'user32::GetWindowLong(p r3, i -16) i .r4'
    IntOp $4 $4 & 0xFF7FFFFF
    System::Call 'user32::SetWindowLong(p r3, i -16, i r4)'
    System::Call 'uxtheme::SetWindowTheme(p r3, w " ", w " ")'
    ; COLORREF ist 0x00BBGGRR: #0B6E75 auf #D5DBDE
    SendMessage $3 ${ASK_PBM_SETBARCOLOR} 0 0x00756E0B
    SendMessage $3 ${ASK_PBM_SETBKCOLOR} 0 0x00DEDBD5

    ; Dialog 254 x 78: Symbol links, Titel und zweite Zeile daneben, darunter ein schmaler Balken
    !insertmacro askPlace $0 1025 10 17 20 20
    !insertmacro askPlace $0 1000 38 15 206 13
    !insertmacro askPlace $0 1002 38 28 206 10
    !insertmacro askPlace $0 1001 10 54 234 4
    ; RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN
    System::Call 'user32::RedrawWindow(p r0, p 0, p 0, i 0x85)'
  ${EndIf}
!macroend

!macro customCheckAppRunning
  !ifndef BUILD_UNINSTALLER
    ${IfNot} ${Silent}
      !insertmacro askStyleBanner
    ${EndIf}
  !endif
  ; Was electron-builder ohne customCheckAppRunning tut (include/allowOnlyOneInstallerInstance.nsh)
  !insertmacro IS_POWERSHELL_AVAILABLE
  !insertmacro _CHECK_APP_RUNNING
!macroend
