; Ergaenzungen zum NSIS-Installer von Tauri (bundle.windows.nsis.installerHooks), ENTSCHEIDE E39, E60 und E63.
; Nur ASCII: makensis liest die Datei ohne BOM.

; Kleines Fortschrittsfenster wie in der Electron-Fassung (ENTSCHEIDE E59, E63): das Plugin SpiderBanner aus
; den nsis-resources von electron-builder (scripts/nsis-plugins.js laedt es mit Pruefsumme nach plugins\).
; Fehlt es (Entwicklungs-Build ohne den Schritt), bleibt das Fenster von NSIS mit der Fortschrittsseite.
!if /FileExists "${__FILEDIR__}\plugins\x86-unicode\SpiderBanner.dll"
  !addplugindir /x86-unicode "${__FILEDIR__}\plugins\x86-unicode"
  !define ASK_HAS_BANNER
!endif

; Eigene Namen: WinMessages.nsh (ueber MUI2) definiert PBM_SETBARCOLOR schon ohne !ifndef
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

; Steuerelemente des Plugins (Dialog 104, Dialog-Einheiten): 1000 Titel, 1002 zweite Zeile, 1001 Balken,
; 1003 Statuszeile von NSIS (leer, SetDetailsPrint none), 1025 Symbol. Wie askStyleBanner in E59.
!macro askStyleBanner
  FindWindow $0 "#32770" "" $HWNDPARENT
  FindWindow $0 "#32770" "" $HWNDPARENT $0
  ${If} $0 <> 0
    SendMessage $HWNDPARENT ${WM_SETTEXT} 0 "STR:ASK"

    ; Update: vom Updater (/UPDATE), von der Electron-Fassung (--updated) oder schon installiert
    StrCpy $6 0
    ${If} $UpdateMode = 1
      StrCpy $6 1
    ${EndIf}
    ${GetParameters} $7
    ClearErrors
    ${GetOptions} $7 "--updated" $8
    ${IfNot} ${Errors}
      StrCpy $6 1
    ${EndIf}
    ReadRegStr $7 SHCTX "${UNINSTKEY}" "DisplayVersion"
    ${If} $7 != ""
    ${OrIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
    ${OrIf} ${FileExists} "$LOCALAPPDATA\Programs\ask-desktop\Uninstall ASK.exe"
      StrCpy $6 1
    ${EndIf}

    ; Nur für den Workflow «Installer-Vorschau»: erkannte Werte in eine Datei schreiben
    ReadEnvStr $9 ASK_INSTALLER_DEBUG
    ${If} $9 != ""
      FileOpen $9 "$9" w
      FileWrite $9 "update=$6 updatemode=$UpdateMode instdir=$INSTDIR version=$7 exe=$INSTDIR\${MAINBINARYNAME}.exe$\r$\n"
      FileClose $9
    ${EndIf}

    CreateFont $1 "Segoe UI" 11 600
    CreateFont $2 "Segoe UI" 9 400
    ${If} $6 = 1
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

; Aufgerufen im ersten Abschnitt der Vorlage (installer.nsi, Section -AskBanner), also vor WebView2 und Install
!macro ASK_BANNER
  ${IfNot} ${Silent}
    SetDetailsPrint none
    !ifdef ASK_HAS_BANNER
      InitPluginsDir
      File "/oname=$PLUGINSDIR\askbanner.ico" "${INSTALLERICON}"
      SpiderBanner::Show /MODERN /ICON "$PLUGINSDIR\askbanner.ico"
      !insertmacro askStyleBanner
    !endif
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Uebergang von der Electron-Fassung (bis 0.3.x): deren Deinstallation still ausfuehren und warten.
  ; Mit --updated behaelt sie das entpackte Python; Einstellungen und Chats (%APPDATA%\ASK) bleiben ohnehin.
  ${If} ${FileExists} "$LOCALAPPDATA\Programs\ask-desktop\Uninstall ASK.exe"
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
