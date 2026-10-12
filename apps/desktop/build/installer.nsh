; Custom NSIS logic injected by electron-builder into the installer script.
;
; 1) Elevate ONCE at launch.
;    electron-builder's built-in assisted-installer flow relaunches the ENTIRE
;    wizard when a non-admin picks "install for all users" (UAC plugin restart).
;    The fresh elevated process re-shows the license page AND auto-skips the
;    install-mode page, so the flow "backtracks" to the agreement. Fix: relaunch
;    ourselves once via ShellExecute "runas" when not admin. The relaunched copy
;    is a plain elevated process (NOT a UAC-plugin "inner instance"), so every
;    page stays in the flow: license -> install mode -> database -> directory ->
;    install -> finish, with full Back/Next navigation at all times.
;
; 2) Database configuration page (BEFORE the installation-directory page).
;    Requires a small patch to electron-builder's assistedInstaller.nsh template
;    (node_modules/.pnpm/app-builder-lib@25.1.8*/node_modules/app-builder-lib/
;    templates/nsis/assistedInstaller.nsh) that inserts the
;    customPageBeforeChangeDir hook between the install-mode page and the
;    directory page. Re-apply that patch after any `pnpm install`.
;
;    Two options: portable (embedded PostgreSQL, downloaded and installed during
;    setup by the customInstall hook below) or an existing PostgreSQL database.
;    For an existing database the user fills in host/port/user/password/database,
;    clicks "测试连接" (runs the packaged `xuanyu.exe db-external` helper extracted
;    to $PLUGINSDIR) and cannot proceed until the connection succeeds. On success
;    the helper writes the connection string into ~\.xuanyu\xuanyu.json
;    (memory.postgres.connectionString), so the app skips its first-run storage
;    wizard and connects to the existing database directly.
;
; 3) Custom installation-directory page (REPLACES MUI_PAGE_DIRECTORY).
;    The stock page shows the parent folder the user picked, while the app is
;    actually installed into "<picked>\${APP_FILENAME}", so users add the folder
;    themselves and get a nested "...\XuanYu Agent\XuanYu Agent". The custom page
;    appends the application folder to the box as soon as a folder is picked.
;    Requires the same assistedInstaller.nsh patch (customDirectoryPage hook).
;
; 4) Portable mode keeps ALL data inside the installation folder.
;    When "便携式" is selected, .onInstSuccess writes ~\.xuanyu\launch.conf
;    with "dataDir=<install>\data", so the desktop app passes XUANYU_DATA_DIR
;    to the engine and every piece of data (embedded PostgreSQL, plugins,
;    skills, mcp, sandboxes, ...) lives under the installation directory.
;    Portable mode is therefore forced to install for the current user only
;    (Program Files would not be writable by the normal user). The embedded
;    PostgreSQL itself is downloaded and installed by the customInstall hook
;    below, which runs the packaged `xuanyu.exe --data-dir <install>\data
;    portable-install` helper; its output is streamed into the install details
;    pane. Requires two more template patches (re-apply after `pnpm install`):
;      - common.nsh: "ShowInstDetails nevershow" -> "show" (makes the details
;        pane visible at all);
;      - installSection.nsh: "SetDetailsPrint none" -> "both" (otherwise the pane
;        stays empty while the application files are extracted, so the installer
;        looks frozen even though the progress bar is moving).
;
; 5) Console encoding: nsExec decodes the helper's byte stream with the system
;    ANSI code page (GBK on zh-CN), while Rust `println!` emits UTF-8 - any
;    Chinese coming from the helper turns into mojibake. The helper therefore
;    prints ASCII/English only, and every Chinese message is emitted by NSIS
;    (DetailPrint is UTF-16 internally, so Chinese and non-ASCII paths are fine).

!include nsDialogs.nsh
!include StrFunc.nsh
!include FileFunc.nsh

!ifndef BUILD_UNINSTALLER
${StrStr} ; trigger StrFunc definition (first use must carry no arguments)

  Var g_dbMode            ; 1 = portable, 2 = existing database
  Var g_dbTestOk          ; 1 = connection verified
  Var g_dbRadioPortable
  Var g_dbRadioExternal
  Var g_dbGroupBox
  Var g_dbLblHost
  Var g_dbLblPort
  Var g_dbLblUser
  Var g_dbLblPass
  Var g_dbLblDb
  Var g_dbEditHost
  Var g_dbEditPort
  Var g_dbEditUser
  Var g_dbEditPass
  Var g_dbEditDb
  Var g_dbBtnTest
  Var g_dbLblResult
  Var g_dbHost
  Var g_dbPort
  Var g_dbUser
  Var g_dbPass
  Var g_dbDb

  !macro customInit
    ${IfNot} ${UAC_IsAdmin}
      ShowWindow $HWNDPARENT ${SW_HIDE}
      ${GetParameters} $R0
      ExecShell "runas" "$EXEPATH" "$R0"
      Quit
    ${EndIf}
    ; Database page state (only the elevated instance reaches this point)
    StrCpy $g_dbMode 1
    StrCpy $g_dbTestOk 0
    StrCpy $g_dbHost ""
    StrCpy $g_dbPort "5432"
    StrCpy $g_dbUser ""
    StrCpy $g_dbPass ""
    StrCpy $g_dbDb "postgres"
  !macroend

  ; Inserted right after the install-mode page and before the directory page
  ; (customPageBeforeChangeDir hook, patched into assistedInstaller.nsh).
  !macro customPageBeforeChangeDir
    Function dbConfigLeaveWithGuard
      Call dbConfigLeave
      ; Portable mode keeps ALL data (embedded PostgreSQL, plugins, skills, ...)
      ; inside the installation folder, so it must be installed for the current
      ; user only: per-machine installs land in Program Files where the normal
      ; user has no write permission and the data folder could not be written.
      ${If} $g_dbMode == 1
      ${AndIf} $installMode == "all"
        MessageBox MB_OK|MB_ICONEXCLAMATION "便携式模式仅支持为当前用户安装。$\r$\n请点击「上一步」返回安装模式页，选择「仅为当前用户安装」，然后再选择便携式数据库。"
        Abort
      ${EndIf}
    FunctionEnd
    Page custom dbConfigCreate dbConfigLeaveWithGuard
  !macroend

  Function dbConfigCreate
    nsDialogs::Create 1018
    Pop $0
    ${If} $0 == error
      Abort
    ${EndIf}

    ; Set the wizard header ourselves: a raw "Page custom" has no PRE callback,
    ; so the header would otherwise keep showing the install-mode page's text.
    ; 1037 / 1038 are the MUI2 header title / subtitle controls, 0x000C = WM_SETTEXT.
    GetDlgItem $0 $HWNDPARENT 1037
    SendMessage $0 0x000C 0 "STR:数据库配置"
    GetDlgItem $0 $HWNDPARENT 1038
    SendMessage $0 0x000C 0 "STR:选择内置的便携式数据库，或填写现有 PostgreSQL 连接信息。"

    ; ---------------------------------------------------------------------
    ; Layout constraint: the page area inside the NSIS wizard is only about
    ; 300 x 135 DLU. Anything placed beyond that is clipped by the wizard
    ; window (invisible, or cut in half), so keep every control inside
    ; x <= 290u and y <= 128u.
    ; ---------------------------------------------------------------------
    ${NSD_CreateRadioButton} 10u 0u 280u 11u "便携式（安装时自动下载内置 PostgreSQL 到安装目录）"
    Pop $g_dbRadioPortable
    ${NSD_OnClick} $g_dbRadioPortable dbConfigOnPortable

    ${NSD_CreateRadioButton} 10u 12u 280u 11u "配置现有数据库（PostgreSQL）"
    Pop $g_dbRadioExternal
    ${NSD_OnClick} $g_dbRadioExternal dbConfigOnExternal

    ${NSD_CreateGroupBox} 10u 26u 280u 102u "现有数据库连接信息"
    Pop $g_dbGroupBox

    ${NSD_CreateLabel} 18u 42u 40u 9u "主机"
    Pop $g_dbLblHost
    ${NSD_CreateText} 60u 41u 84u 12u ""
    Pop $g_dbEditHost
    ${NSD_CreateLabel} 150u 42u 40u 9u "端口"
    Pop $g_dbLblPort
    ${NSD_CreateText} 192u 41u 62u 12u ""
    Pop $g_dbEditPort

    ${NSD_CreateLabel} 18u 56u 40u 9u "用户名"
    Pop $g_dbLblUser
    ${NSD_CreateText} 60u 55u 194u 12u ""
    Pop $g_dbEditUser

    ${NSD_CreateLabel} 18u 70u 40u 9u "密码"
    Pop $g_dbLblPass
    ${NSD_CreatePassword} 60u 69u 194u 12u ""
    Pop $g_dbEditPass

    ${NSD_CreateLabel} 18u 84u 40u 9u "数据库"
    Pop $g_dbLblDb
    ${NSD_CreateText} 60u 83u 194u 12u ""
    Pop $g_dbEditDb

    ${NSD_CreateButton} 60u 100u 64u 13u "测试连接"
    Pop $g_dbBtnTest
    ${NSD_OnClick} $g_dbBtnTest dbConfigOnTest

    ; own row across the group box: a connection result can be long
    ${NSD_CreateLabel} 18u 115u 258u 10u ""
    Pop $g_dbLblResult

    ; restore values previously entered (so Back/Next navigation keeps them)
    ${NSD_SetText} $g_dbEditHost $g_dbHost
    ${NSD_SetText} $g_dbEditPort $g_dbPort
    ${NSD_SetText} $g_dbEditUser $g_dbUser
    ${NSD_SetText} $g_dbEditPass $g_dbPass
    ${NSD_SetText} $g_dbEditDb $g_dbDb

    ${If} $g_dbMode == 2
      ${NSD_Check} $g_dbRadioExternal
      ${NSD_Uncheck} $g_dbRadioPortable
      Call dbConfigShowExternal
      ${If} $g_dbTestOk == 1
        ${NSD_SetText} $g_dbLblResult "连接已通过验证，可继续下一步。"
      ${EndIf}
    ${Else}
      ${NSD_Check} $g_dbRadioPortable
      ${NSD_Uncheck} $g_dbRadioExternal
      Call dbConfigHideExternal
    ${EndIf}

    ; extract the connection-test helper now (it only runs when the user tests).
    ; NOTE: this page runs BEFORE the directory page, so $INSTDIR is not final
    ; yet - stay inside $PLUGINSDIR and do not touch the install location.
    SetOutPath "$PLUGINSDIR"
    File "/oname=xuanyu.exe" "${__FILEDIR__}\..\..\..\target\release\xuanyu.exe"

    nsDialogs::Show
  FunctionEnd

  Function dbConfigLeave
    ${NSD_GetText} $g_dbEditHost $g_dbHost
    ${NSD_GetText} $g_dbEditPort $g_dbPort
    ${NSD_GetText} $g_dbEditUser $g_dbUser
    ${NSD_GetText} $g_dbEditPass $g_dbPass
    ${NSD_GetText} $g_dbEditDb $g_dbDb
    ${If} $g_dbMode == 2
    ${AndIf} $g_dbTestOk != 1
      MessageBox MB_OK|MB_ICONEXCLAMATION "请先点击「测试连接」并确认连接成功，才能继续下一步。"
      Abort
    ${EndIf}
  FunctionEnd

  Function dbConfigOnPortable
    StrCpy $g_dbMode 1
    ${NSD_Check} $g_dbRadioPortable
    ${NSD_Uncheck} $g_dbRadioExternal
    Call dbConfigHideExternal
  FunctionEnd

  Function dbConfigOnExternal
    StrCpy $g_dbMode 2
    ${NSD_Check} $g_dbRadioExternal
    ${NSD_Uncheck} $g_dbRadioPortable
    Call dbConfigShowExternal
  FunctionEnd

  Function dbConfigShowExternal
    ShowWindow $g_dbGroupBox ${SW_SHOW}
    ShowWindow $g_dbLblHost ${SW_SHOW}
    ShowWindow $g_dbLblPort ${SW_SHOW}
    ShowWindow $g_dbLblUser ${SW_SHOW}
    ShowWindow $g_dbLblPass ${SW_SHOW}
    ShowWindow $g_dbLblDb ${SW_SHOW}
    ShowWindow $g_dbEditHost ${SW_SHOW}
    ShowWindow $g_dbEditPort ${SW_SHOW}
    ShowWindow $g_dbEditUser ${SW_SHOW}
    ShowWindow $g_dbEditPass ${SW_SHOW}
    ShowWindow $g_dbEditDb ${SW_SHOW}
    ShowWindow $g_dbBtnTest ${SW_SHOW}
    ShowWindow $g_dbLblResult ${SW_SHOW}
  FunctionEnd

  Function dbConfigHideExternal
    ShowWindow $g_dbGroupBox ${SW_HIDE}
    ShowWindow $g_dbLblHost ${SW_HIDE}
    ShowWindow $g_dbLblPort ${SW_HIDE}
    ShowWindow $g_dbLblUser ${SW_HIDE}
    ShowWindow $g_dbLblPass ${SW_HIDE}
    ShowWindow $g_dbLblDb ${SW_HIDE}
    ShowWindow $g_dbEditHost ${SW_HIDE}
    ShowWindow $g_dbEditPort ${SW_HIDE}
    ShowWindow $g_dbEditUser ${SW_HIDE}
    ShowWindow $g_dbEditPass ${SW_HIDE}
    ShowWindow $g_dbEditDb ${SW_HIDE}
    ShowWindow $g_dbBtnTest ${SW_HIDE}
    ShowWindow $g_dbLblResult ${SW_HIDE}
  FunctionEnd

  Function dbConfigOnTest
    ${NSD_GetText} $g_dbEditHost $g_dbHost
    ${NSD_GetText} $g_dbEditPort $g_dbPort
    ${NSD_GetText} $g_dbEditUser $g_dbUser
    ${NSD_GetText} $g_dbEditPass $g_dbPass
    ${NSD_GetText} $g_dbEditDb $g_dbDb

    ${If} $g_dbHost == ""
      ${NSD_SetText} $g_dbLblResult "请填写主机地址。"
      Return
    ${EndIf}
    ${If} $g_dbPort == ""
      StrCpy $g_dbPort "5432"
    ${EndIf}
    ${If} $g_dbUser == ""
      ${NSD_SetText} $g_dbLblResult "请填写用户名。"
      Return
    ${EndIf}
    ${If} $g_dbDb == ""
      StrCpy $g_dbDb "postgres"
    ${EndIf}

    ; double quotes in any field would break the command line -> reject them
    ${StrStr} $0 $g_dbHost '"'
    ${If} $0 != ""
      ${NSD_SetText} $g_dbLblResult "字段不能包含双引号（$\"）。"
      Return
    ${EndIf}
    ${StrStr} $0 $g_dbPort '"'
    ${If} $0 != ""
      ${NSD_SetText} $g_dbLblResult "字段不能包含双引号（$\"）。"
      Return
    ${EndIf}
    ${StrStr} $0 $g_dbUser '"'
    ${If} $0 != ""
      ${NSD_SetText} $g_dbLblResult "字段不能包含双引号（$\"）。"
      Return
    ${EndIf}
    ${StrStr} $0 $g_dbPass '"'
    ${If} $0 != ""
      ${NSD_SetText} $g_dbLblResult "字段不能包含双引号（$\"）。"
      Return
    ${EndIf}
    ${StrStr} $0 $g_dbDb '"'
    ${If} $0 != ""
      ${NSD_SetText} $g_dbLblResult "字段不能包含双引号（$\"）。"
      Return
    ${EndIf}

    StrCpy $g_dbTestOk 0
    ${NSD_SetText} $g_dbLblResult "正在测试连接，请稍候..."
    ; drop any stale result so a helper that never ran is detected
    Delete "$PLUGINSDIR\db-result.txt"
    nsExec::ExecToStack '"$PLUGINSDIR\xuanyu.exe" db-external --host "$g_dbHost" --port "$g_dbPort" --username "$g_dbUser" --password "$g_dbPass" --database "$g_dbDb" --result-file "$PLUGINSDIR\db-result.txt"'
    Pop $0 ; exit code
    Pop $0 ; the single JSON output line (unused; the result file carries the message)

    ; The helper writes UTF-16LE without a BOM. Reading it with plain FileRead
    ; would decode the bytes with the system ANSI code page and turn every
    ; Chinese character into mojibake, so use FileReadUTF16LE (this installer
    ; is built as Unicode). File format: "OK <version>" or "ERR <message>".
    StrCpy $5 ""
    FileOpen $4 "$PLUGINSDIR\db-result.txt" r
    ${If} $4 != ""
      FileReadUTF16LE $4 $5
      FileClose $4
    ${EndIf}

    ${If} $5 == ""
      StrCpy $g_dbTestOk 0
      ${NSD_SetText} $g_dbLblResult "连接失败：无法启动连接测试工具。"
      Return
    ${EndIf}

    StrCpy $6 $5 2
    ${If} $6 == "OK"
      StrCpy $g_dbTestOk 1
      StrCpy $6 $5 "" 3 ; strip the "OK " prefix
      ${NSD_SetText} $g_dbLblResult "连接成功：$6"
    ${Else}
      StrCpy $g_dbTestOk 0
      StrCpy $6 $5 "" 4 ; strip the "ERR " prefix
      ${NSD_SetText} $g_dbLblResult "连接失败：$6"
    ${EndIf}
  FunctionEnd

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

  ; ---------------------------------------------------------------------
  ; Write the launch marker that tells the desktop app where its data lives.
  ;
  ;   portable: ~\.xuanyu\launch.conf  -> "mode=portable" + "dataDir=<install>\data"
  ;             ALL data (embedded PostgreSQL, plugins, skills, ...) stays inside
  ;             the installation folder, so uninstalling removes everything.
  ;   existing database: the marker is removed -> the app falls back to the
  ;             default ~\.xuanyu data directory.
  ;
  ; The marker is written by the installer and read by the Electron main process
  ; on startup. Portable mode is forced to install for the current user only
  ; (see dbConfigLeaveWithGuard), so this callback runs WITHOUT elevation and
  ; $PROFILE reliably points at the installing user's home directory.
  ;
  ; NOTE: this installer is built as Unicode, so FileWrite emits UTF-16LE. The
  ; Electron reader must detect and decode UTF-16LE (it also tolerates UTF-8).
  ; ---------------------------------------------------------------------
  Function .onInstSuccess
    CreateDirectory "$PROFILE\.xuanyu"
    ${If} $g_dbMode == 1
      FileOpen $0 "$PROFILE\.xuanyu\launch.conf" w
      FileWrite $0 "mode=portable$\r$\n"
      FileWrite $0 "dataDir=$INSTDIR\data$\r$\n"
      FileClose $0
    ${Else}
      Delete "$PROFILE\.xuanyu\launch.conf"
    ${EndIf}
  FunctionEnd

  ; ---------------------------------------------------------------------
  ; Portable mode: download and install the embedded PostgreSQL during setup.
  ;
  ; Runs at the end of the install section (customInstall hook), so $INSTDIR is
  ; final. The packaged xuanyu.exe (already extracted to $PLUGINSDIR by the
  ; database page) downloads PostgreSQL 17.6 + pgvector, extracts them into
  ; $INSTDIR\data\store\postgres and runs initdb. Progress is streamed line by
  ; line into the install details pane (nsExec::ExecToLog).
  ;
  ; The helper starts the server, creates the database/extension, and leaves it
  ; running, so the app connects immediately on first launch. The engine owns the
  ; lifecycle from then on (it starts the instance if it is down and stops it on
  ; shutdown), and the installer cleans up leftover processes before reinstall.
  ; On success it also writes the connection string into the global config
  ; (~/.xuanyu/xuanyu.json, memory.backend + memory.postgres.connectionString),
  ; so the app connects on first launch without asking for database settings.
  ; A failure does not abort the setup: the engine retries the download on its
  ; first start.
  ; ---------------------------------------------------------------------
  ; 中文提示必须留在 NSIS 侧（DetailPrint 走 UTF-16，显示正常）。nsExec 把子进程输出
  ; 按系统 ANSI 代码页解码，而 xuanyu.exe 输出 UTF-8，所以辅助程序只打印英文，
  ; 目标路径也由这里打印（NSIS 能正确处理含中文的路径）。
  ; ---------------------------------------------------------------------
  !macro customInstall
    ${If} $g_dbMode == 1
      SetDetailsPrint both
      DetailPrint "便携式模式：安装内置 PostgreSQL 到 $INSTDIR\data..."
      DetailPrint "已下载过的安装包会从缓存目录 %LOCALAPPDATA%\XuanYu\cache\postgres 直接解压，不再重复下载。"
      nsExec::ExecToLog '"$PLUGINSDIR\xuanyu.exe" --data-dir "$INSTDIR\data" portable-install'
      Pop $0
      ${If} $0 == 0
        DetailPrint "内置 PostgreSQL 安装完成。"
      ${Else}
        DetailPrint "内置 PostgreSQL 安装失败，退出码 $0。"
        MessageBox MB_OK|MB_ICONEXCLAMATION "内置数据库未能自动安装（退出码 $0）。$\r$\n首次启动 XuanYu Agent 时会自动重试下载安装，请确认网络可用。"
      ${EndIf}
    ${EndIf}
  !macroend

!endif
