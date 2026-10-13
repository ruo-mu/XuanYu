; Custom NSIS logic injected by electron-builder into the installer script.
;
; 1) Elevate ONCE at launch.
;    electron-builder's built-in assisted-installer flow relaunches the ENTIRE
;    wizard when a non-admin picks "install for all users" (UAC plugin restart).
;    The fresh elevated process re-shows the license page AND auto-skips the
;    install-mode page, so the flow "backtracks" to the agreement. Fix: relaunch
;    ourselves once via ShellExecute "runas" when not admin. The relaunched copy
;    is a plain elevated process (NOT a UAC-plugin "inner instance"), so every
;    remaining page stays in the flow with full Back/Next navigation at all times.
;
; 2) Custom installation-directory page (REPLACES MUI_PAGE_DIRECTORY).
;    The stock page shows the parent folder the user picked, while the app is
;    actually installed into "<picked>\${APP_FILENAME}", so users add the folder
;    themselves and get a nested "...\XuanYu Agent\XuanYu Agent". The custom page
;    appends the application folder to the box as soon as a folder is picked.
;    Requires the customDirectoryPage patch to electron-builder's
;    assistedInstaller.nsh template, applied by scripts/patch-nsis-templates.mjs
;    (re-apply after any `pnpm install`).

!include nsDialogs.nsh
!include StrFunc.nsh
!include FileFunc.nsh

!ifndef BUILD_UNINSTALLER
${StrStr} ; trigger StrFunc definition (first use must carry no arguments)

  !macro customInit
    ${IfNot} ${UAC_IsAdmin}
      ShowWindow $HWNDPARENT ${SW_HIDE}
      ${GetParameters} $R0
      ExecShell "runas" "$EXEPATH" "$R0"
      Quit
    ${EndIf}
  !macroend

  ; ---------------------------------------------------------------------
  ; Custom installation-directory page.
  ;
  ; electron-builder's stock flow (MUI_PAGE_DIRECTORY + instFilesPre) appends
  ; "${APP_FILENAME}" only when the install page is entered, so the box shows
  ; "D:\software" while the app really lands in "D:\software\XuanYu Agent".
  ; Users who don't know that add the folder themselves and end up with a
  ; nested "...\XuanYu Agent\XuanYu Agent". This page keeps the box in sync
  ; with the real target: browsing to a parent folder appends the application
  ; folder immediately.
  ; ---------------------------------------------------------------------
  Var g_dirEdit
  Var g_dirBrowse
  Var g_dirSpaceReq
  Var g_dirSpaceAvail

  ; Append ${APP_FILENAME} to $INSTDIR unless it is already the last element.
  ; Also normalizes "D:\software\" to "D:\software" (drive roots stay intact).
  Function dirPageEnsureAppFolder
    StrCpy $0 $INSTDIR 1 -1
    ${If} $0 == "\"
      StrLen $0 $INSTDIR
      ${If} $0 > 3
        StrCpy $INSTDIR $INSTDIR -1
      ${EndIf}
    ${EndIf}
    ${StrStr} $0 $INSTDIR "${APP_FILENAME}"
    ${If} $0 == ""
      StrCpy $INSTDIR "$INSTDIR\${APP_FILENAME}"
    ${EndIf}
  FunctionEnd

  Function dirPageSetSpace
    ; ${ESTIMATED_SIZE} is in KB; MUI shows MB with one decimal, do the same
    IntOp $2 ${ESTIMATED_SIZE} * 10
    IntOp $2 $2 / 1024
    StrCpy $3 $2 -1
    StrCpy $4 $2 1 -1
    ${NSD_SetText} $g_dirSpaceReq "所需空间：$3.$4 MB"

    ${GetRoot} "$INSTDIR" $0
    ${DriveSpace} "$0\" "/D=F /S=M" $1
    ${If} $1 == ""
      ${NSD_SetText} $g_dirSpaceAvail "可用空间：未知"
      Return
    ${EndIf}

    IntOp $5 ${ESTIMATED_SIZE} / 1024
    ${If} $1 >= 1024
      IntOp $6 $1 * 10
      IntOp $6 $6 / 1024
      StrCpy $7 $6 -1
      StrCpy $8 $6 1 -1
      StrCpy $9 "$7.$8 GB"
    ${Else}
      StrCpy $9 "$1 MB"
    ${EndIf}
    ${If} $1 < $5
      StrCpy $9 "$9（空间不足）"
    ${EndIf}
    ${NSD_SetText} $g_dirSpaceAvail "可用空间：$9"
  FunctionEnd

  Function dirPageCreate
    ; The "skip this page on update" guard was removed: it relied on ${isUpdated}
    ; (StdUtils plugin), which is only registered inside the customDirectoryPage
    ; macro expansion and not available at this file's scope. The directory
    ; dialog still works for both install and update; keeping the existing
    ; location is handled by dirPageEnsureAppFolder / dirPageLeave.
    nsDialogs::Create 1018
    Pop $0
    ${If} $0 == error
      Abort
    ${EndIf}

    Call dirPageEnsureAppFolder

    ; raw "Page custom" has no PRE callback, so set the wizard header here.
    ; 1037 / 1038 are the MUI2 header title / subtitle controls.
    GetDlgItem $1 $HWNDPARENT 1037
    SendMessage $1 0x000C 0 "STR:选定安装位置"
    GetDlgItem $1 $HWNDPARENT 1038
    SendMessage $1 0x000C 0 "STR:选定 XuanYu Agent 要安装的文件夹。"

    ; page area is about 300 x 135 DLU - keep everything inside x <= 290u / y <= 128u
    ${NSD_CreateLabel} 0u 0u 280u 32u "Setup 将安装 XuanYu Agent 到下面的文件夹。$\r$\n若要安装到其他位置，请单击「浏览...」选择文件夹，XuanYu Agent 会自动安装到所选文件夹下的 ${APP_FILENAME} 子文件夹中。"

    ${NSD_CreateGroupBox} 0u 44u 280u 34u "目标文件夹"
    ${NSD_CreateText} 8u 56u 224u 12u "$INSTDIR"
    Pop $g_dirEdit
    ${NSD_CreateButton} 238u 55u 36u 13u "浏览..."
    Pop $g_dirBrowse
    ${NSD_OnClick} $g_dirBrowse dirPageBrowse

    ${NSD_CreateLabel} 0u 86u 280u 9u ""
    Pop $g_dirSpaceReq
    ${NSD_CreateLabel} 0u 98u 280u 9u ""
    Pop $g_dirSpaceAvail

    Call dirPageSetSpace
    nsDialogs::Show
  FunctionEnd

  Function dirPageBrowse
    ${NSD_GetText} $g_dirEdit $0
    nsDialogs::SelectFolderDialog "选择安装位置" "$0"
    Pop $0
    ${If} $0 == error
      Return
    ${EndIf}
    ${If} $0 == ""
      Return
    ${EndIf}

    ; the user picks the PARENT folder, so append the application folder at once
    StrCpy $1 $0 1 -1
    ${If} $1 == "\"
      StrLen $1 $0
      ${If} $1 > 3
        StrCpy $0 $0 -1
      ${EndIf}
    ${EndIf}
    ${StrStr} $1 $0 "${APP_FILENAME}"
    ${If} $1 == ""
      StrCpy $0 "$0\${APP_FILENAME}"
    ${EndIf}

    StrCpy $INSTDIR $0
    ${NSD_SetText} $g_dirEdit "$INSTDIR"
    Call dirPageSetSpace
  FunctionEnd

  Function dirPageLeave
    ${NSD_GetText} $g_dirEdit $INSTDIR
    ${If} $INSTDIR == ""
      MessageBox MB_OK|MB_ICONEXCLAMATION "请选择安装位置。"
      Abort
    ${EndIf}
    Call dirPageEnsureAppFolder
  FunctionEnd

  ; Inserted in assistedInstaller.nsh in place of MUI_PAGE_DIRECTORY.
  ; ${isUpdated} expands to a StdUtils plugin call, so the check has to live
  ; inside this macro: macros are expanded from assistedInstaller.nsh, where
  ; electron-builder has already registered the StdUtils plugin directory.
  !macro customDirectoryPage
    Page custom dirPageCreate dirPageLeave
  !macroend

!endif
