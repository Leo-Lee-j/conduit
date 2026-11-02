Unicode true
!include "MUI2.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

!ifndef APP_VERSION
  !error "Pass /DAPP_VERSION=<version>"
!endif
!ifndef APP_EXE
  !error "Pass /DAPP_EXE=<absolute executable path>"
!endif
!ifndef APP_ICON
  !error "Pass /DAPP_ICON=<absolute icon path>"
!endif
!ifndef UPDATER_EXE
  !error "Pass /DUPDATER_EXE=<absolute update helper path>"
!endif
!ifndef OUTPUT_FILE
  !error "Pass /DOUTPUT_FILE=<absolute installer path>"
!endif

Name "Conduit"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Conduit"
InstallDirRegKey HKCU "Software\Conduit" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetCompressorDictSize 32
BrandingText "Conduit ${APP_VERSION}"
Icon "${APP_ICON}"
UninstallIcon "${APP_ICON}"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "Conduit"
VIAddVersionKey /LANG=1033 "FileDescription" "Conduit Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" ""

!define MUI_ICON "${APP_ICON}"
!define MUI_UNICON "${APP_ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\conduit.exe"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "Conduit requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "Conduit" SEC_APP
  SectionIn RO
  SetOutPath "$INSTDIR"
  ClearErrors
  File /oname=conduit.exe "${APP_EXE}"
  File /oname=conduit-updater.exe "${UPDATER_EXE}"
  !ifdef RUNTIME_DIR
    File "${RUNTIME_DIR}\*.dll"
  !endif
  IfErrors install_failed
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Conduit"
  CreateShortcut "$SMPROGRAMS\Conduit\Conduit.lnk" "$INSTDIR\conduit.exe" "" "$INSTDIR\conduit.exe" 0
  CreateShortcut "$SMPROGRAMS\Conduit\Uninstall Conduit.lnk" "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\Conduit" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "DisplayName" "Conduit"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "DisplayIcon" "$INSTDIR\conduit.exe,0"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "UninstallString" '$\"$INSTDIR\uninstall.exe$\"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "QuietUninstallString" '$\"$INSTDIR\uninstall.exe$\" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit" "NoRepair" 1
  IfErrors install_failed
  FileOpen $0 "$INSTDIR\conduit.install" w
  FileWrite $0 "${APP_VERSION}"
  FileClose $0
  IfErrors install_failed
  Goto install_done
install_failed:
  SetErrorLevel 1
  Quit
install_done:
SectionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "Uninstall"
  Delete "$INSTDIR\conduit.exe"
  Delete "$INSTDIR\conduit-updater.exe"
  Delete "$INSTDIR\conduit.install"
  Delete "$INSTDIR\.conduit-running.lock"
  Delete "$INSTDIR\.conduit-update.lock"
  !ifdef RUNTIME_DIR
    !include "${RUNTIME_UNINSTALL}"
  !endif
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Conduit\Conduit.lnk"
  Delete "$SMPROGRAMS\Conduit\Uninstall Conduit.lnk"
  RMDir "$SMPROGRAMS\Conduit"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Conduit"
  DeleteRegKey HKCU "Software\Conduit"
  ; User data in $LOCALAPPDATA\Conduit is intentionally retained.
SectionEnd
