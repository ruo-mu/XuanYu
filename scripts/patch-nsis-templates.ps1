# Re-applies the hand-patched NSIS templates that electron-builder needs for
# the custom DB page, custom directory page and visible install progress.
#
# These patches live inside node_modules, so every `pnpm install` wipes them.
# apps/desktop runs this via its "postinstall" script so they are restored
# automatically. It is idempotent: already-patched files are left untouched.
#
# Patches applied:
#   1. assistedInstaller.nsh -> insert the customPageBeforeChangeDir (DB page)
#      hook between the install-mode page and the directory page.
#   2. assistedInstaller.nsh -> expand customDirectoryPage when defined,
#      otherwise fall back to MUI_PAGE_DIRECTORY.
#   3. common.nsh -> ShowInstDetails show (show the install details pane).
#   4. installSection.nsh -> SetDetailsPrint both (visible DB download progress).
#
# For details see apps/desktop/build/installer.nsh header comment.

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot

$tplDir = Get-ChildItem (Join-Path $repoRoot "node_modules\.pnpm") -Directory 2>$null |
    Where-Object { $_.Name -like "app-builder-lib@*" } |
    ForEach-Object { Join-Path $_.FullName "node_modules\app-builder-lib\templates\nsis" } |
    Where-Object { Test-Path $_ } |
    Select-Object -First 1

if (-not $tplDir) {
    Write-Host "WARN: app-builder-lib NSIS templates not found; skipping NSIS patches." -ForegroundColor Yellow
    exit 0
}

$assisted = Join-Path $tplDir "assistedInstaller.nsh"
$common   = Join-Path $tplDir "common.nsh"
$install  = Join-Path $tplDir "installSection.nsh"

function Set-FileText([string]$path, [string]$old, [string]$new, [string]$marker) {
    if (-not (Test-Path $path)) {
        Write-Host "skip  $($path): missing" -ForegroundColor Yellow
        return
    }
    # The npm package templates use LF, while this script (and therefore the
    # here-strings below) may be checked out with CRLF. Normalise both sides to
    # LF before comparing so the patches apply regardless of line endings.
    $text = (Get-Content $path -Raw) -replace "`r`n", "`n"
    $old = $old -replace "`r`n", "`n"
    $new = $new -replace "`r`n", "`n"
    if ($text.Contains($new)) {
        Write-Host "ok    $([System.IO.Path]::GetFileName($path)): already patched ($marker)"
        return
    }
    if (-not $text.Contains($old)) {
        Write-Host "WARN  $($path): expected text not found; patch ($marker) not applied" -ForegroundColor Yellow
        return
    }
    # Write UTF-8 without a BOM, matching the untouched npm templates.
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($path, $text.Replace($old, $new), $encoding)
    Write-Host "patch $([System.IO.Path]::GetFileName($path)): $marker"
}

# 1. customPageBeforeChangeDir (DB config page) hook after the install-mode page.
$hookIn = @'
  !ifndef INSTALL_MODE_PER_ALL_USERS
    !insertmacro PAGE_INSTALL_MODE
  !endif
'@
$hookOut = @'
  !ifndef INSTALL_MODE_PER_ALL_USERS
    !insertmacro PAGE_INSTALL_MODE
  !endif

  # XuanYu patch: wire the customPageBeforeChangeDir (database config) hook.
  # customDirectoryPage is handled by electron-builder automatically; this
  # custom hook is NOT and must be inserted here manually.
  !ifmacrodef customPageBeforeChangeDir
    !insertmacro customPageBeforeChangeDir
  !endif
'@
Set-FileText $assisted $hookIn $hookOut "customPageBeforeChangeDir (database config) hook"

# 2. customDirectoryPage expansion vs. stock MUI_PAGE_DIRECTORY.
#    skipPageIfUpdated and the instFilesPre PRE callback (which appends
#    "${APP_FILENAME}" to $INSTDIR) are only needed by the stock page; the custom
#    page handles that itself. Keep them inside the !else branch so defining
#    MUI_PAGE_CUSTOMFUNCTION_PRE here cannot collide with skipPageIfUpdated's
#    ("MUI_PAGE_CUSTOMFUNCTION_PRE already defined!").
$dirIn = @'
    !include StrContains.nsh

    !insertmacro skipPageIfUpdated
    !insertmacro MUI_PAGE_DIRECTORY

    # pageDirectory leave doesn't work (it seems because $INSTDIR is set after custom leave function)
    # so, we use instfiles pre
    !define MUI_PAGE_CUSTOMFUNCTION_PRE instFilesPre

    # sanitize the MUI_PAGE_DIRECTORY result to make sure it has a application name sub-folder
    Function instFilesPre
      ${StrContains} $0 "${APP_FILENAME}" $INSTDIR
      ${If} $0 == ""
        StrCpy $INSTDIR "$INSTDIR\${APP_FILENAME}"
      ${endIf}
    FunctionEnd
'@
$dirOut = @'
    # XuanYu patch: use the customDirectoryPage when defined, else the stock page.
    # StrContains.nsh is pulled in only by the stock page (instFilesPre uses it);
    # keeping it inside !else avoids a 6010 "StrContains not referenced" warning
    # when the custom directory page is active.
    !ifmacrodef customDirectoryPage
      !insertmacro customDirectoryPage
    !else
      !include StrContains.nsh
      !insertmacro skipPageIfUpdated
      !insertmacro MUI_PAGE_DIRECTORY

      # pageDirectory leave doesn't work (it seems because $INSTDIR is set after custom leave function)
      # so, we use instfiles pre
      !define MUI_PAGE_CUSTOMFUNCTION_PRE instFilesPre

      # sanitize the MUI_PAGE_DIRECTORY result to make sure it has a application name sub-folder
      Function instFilesPre
        ${StrContains} $0 "${APP_FILENAME}" $INSTDIR
        ${If} $0 == ""
          StrCpy $INSTDIR "$INSTDIR\${APP_FILENAME}"
        ${endIf}
      FunctionEnd
    !endif
'@
Set-FileText $assisted $dirIn $dirOut "customDirectoryPage conditional block (instFilesPre inside !else)"

# 3. Show the install details pane.
Set-FileText $common "ShowInstDetails nevershow" "ShowInstDetails show" "ShowInstDetails show"

# 4. Visible extraction progress (otherwise the pane stays blank while DB setup runs).
Set-FileText $install "SetDetailsPrint none" "SetDetailsPrint both" "SetDetailsPrint both"

Write-Host "NSIS template patches applied."