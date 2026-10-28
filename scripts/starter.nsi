; Starter der einzelnen EXE ASK-<Version>.exe (ENTSCHEIDE E18), gebaut von scripts/build-portable.js.
; Erster Start: App mit Startbild nach %LOCALAPPDATA%\ASK-Desktop\app\<BUILD_ID> entpacken, über
; einen Ordner .neu, damit ein abgebrochenes Entpacken nie als fertig gilt. Spätere Starts rufen
; die App direkt von dort auf, ohne Fenster. Ältere Stände räumt die App weg (src/main/portable.js).
; Defines: BUILD_ID, VERSION, APP_DIR, OUT_FILE, SPLASH, ICON

Unicode true
!include LogicLib.nsh
!include FileFunc.nsh

Name "ASK"
OutFile "${OUT_FILE}"
Icon "${ICON}"
RequestExecutionLevel user
SetCompressor zlib
; Eine Prüfsumme über die ganze EXE würde jeden Start um Sekunden verzögern.
CRCCheck off
WindowIcon off
AutoCloseWindow true
ShowInstDetails nevershow
InstallDir "$LOCALAPPDATA\ASK-Desktop\app\${BUILD_ID}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey ProductName "ASK"
VIAddVersionKey FileDescription "ASK"
VIAddVersionKey CompanyName "TF Bern"
VIAddVersionKey LegalCopyright "TF Bern"
VIAddVersionKey FileVersion "${VERSION}"
; Der Test liest daraus den Ordnernamen (tests/e2e/portable-test.js).
VIAddVersionKey ProductVersion "${BUILD_ID}"

Function StartApp
  ${GetParameters} $R0
  SetOutPath $INSTDIR
  Exec '"$INSTDIR\ASK.exe" $R0'
FunctionEnd

Function .onInit
  ${If} ${FileExists} "$INSTDIR\ASK.exe"
    Call StartApp
    Quit
  ${EndIf}
  ; Ein zweiter Doppelklick während des Entpackens: Der erste Starter ruft die App auf.
  System::Call 'kernel32::CreateMutex(p 0, i 0, t "ASK-Starter-${BUILD_ID}") p .r1 ?e'
  Pop $R1
  ${If} $R1 = 183
    Quit
  ${EndIf}
FunctionEnd

Function .onGUIInit
  InitPluginsDir
  File /oname=$PLUGINSDIR\splash.bmp "${SPLASH}"
  BgImage::SetBg $PLUGINSDIR\splash.bmp
  BgImage::Redraw
FunctionEnd

Section
  HideWindow
  RMDir /r "$INSTDIR.neu"
  SetOutPath "$INSTDIR.neu"
  File /r "${APP_DIR}\*.*"
  SetOutPath "$LOCALAPPDATA\ASK-Desktop\app"
  ClearErrors
  Rename "$INSTDIR.neu" "$INSTDIR"
  ${If} ${Errors}
  ${AndIfNot} ${FileExists} "$INSTDIR\ASK.exe"
    BgImage::Destroy
    MessageBox MB_ICONSTOP "ASK konnte nicht entpackt werden. Bitte die Datei nochmals starten.$\n$\n$INSTDIR"
    Quit
  ${EndIf}
  RMDir /r "$INSTDIR.neu"
  Call StartApp
  ; Startbild stehen lassen, bis das Fenster der App erscheint
  Sleep 2000
  BgImage::Destroy
SectionEnd
