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
!ifndef APP_LICENSE
  !error "Pass /DAPP_LICENSE=<absolute license path>"
!endif
!ifndef APP_NOTICE
  !error "Pass /DAPP_NOTICE=<absolute notice path>"
!endif
!ifndef UPDATER_EXE
  !error "Pass /DUPDATER_EXE=<absolute update helper path>"
!endif
!ifndef LAUNCHER_EXE
  !error "Pass /DLAUNCHER_EXE=<absolute launcher path>"
!endif
!ifndef OUTPUT_FILE
  !error "Pass /DOUTPUT_FILE=<absolute installer path>"
!endif

!define APP_FILENAME "conduit-${APP_VERSION}.exe"

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
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright (C) 2026 Leo-Lee-j and Conduit contributors"

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
  File /oname=${APP_FILENAME} "${APP_EXE}"
  ; A short-lived launcher preserves existing shortcuts and old updater restarts.
  File /oname=conduit.exe "${LAUNCHER_EXE}"
  File /oname=conduit-updater.exe "${UPDATER_EXE}"
  File /oname=LICENSE "${APP_LICENSE}"
  File /oname=NOTICE "${APP_NOTICE}"
  !ifdef RUNTIME_DIR
    File "${RUNTIME_DIR}\*.dll"
  !endif
  IfErrors install_failed
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateDirectory "$SMPROGRAMS\Conduit"
  CreateShortcut "$SMPROGRAMS\Conduit\Conduit.lnk" "$INSTDIR\conduit.exe" "" "$INSTDIR\conduit.exe" 0
  CreateShortcut "$SMPROGRAMS\Conduit\Uninstall Conduit.lnk" "$INSTDIR\uninstall.exe"
  ; Explorer-created desktop links can retain the old shell item's icon.
  ; Replace an existing link, but respect users who removed it (or never made one).
  IfFileExists "$DESKTOP\Conduit.lnk" 0 desktop_shortcut_done
  CreateShortcut "$DESKTOP\Conduit.lnk" "$INSTDIR\conduit.exe" "" "$INSTDIR\conduit.exe" 0
desktop_shortcut_done:
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
  ; Remove obsolete versioned apps only after the new installation is complete.
  ; Keep the current app and updater; a locked old app can be cleaned up later.
  FindFirst $0 $1 "$INSTDIR\conduit-*.exe"
cleanup_versions:
  StrCmp $1 "" cleanup_versions_done
  StrCmp $1 "${APP_FILENAME}" cleanup_versions_next
  StrCmp $1 "conduit-updater.exe" cleanup_versions_next
  Delete "$INSTDIR\$1"
cleanup_versions_next:
  FindNext $0 $1
  Goto cleanup_versions
cleanup_versions_done:
  FindClose $0
  ; Shortcut paths stay stable. Refresh their icons, including silent upgrades.
  ; SHCNE_ASSOCCHANGED, SHCNF_IDLIST
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "Uninstall"
  Delete "$INSTDIR\conduit.exe"
  Delete "$INSTDIR\conduit-*.exe"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\NOTICE"
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
