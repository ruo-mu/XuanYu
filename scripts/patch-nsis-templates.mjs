#!/usr/bin/env node
// 重打 electron-builder 的 NSIS 模板补丁。这些模板位于 node_modules，每次 `pnpm install`
// 都会被还原，因此由 apps/desktop 的 postinstall 调用本脚本重新应用。
//
// 唯一补丁：assistedInstaller.nsh 在 allowToChangeInstallationDirectory 分支里优先展开
// customDirectoryPage（apps/desktop/build/installer.nsh 定义的自定义安装目录页），
// 未定义时回退到 electron-builder 原生页面。原生模板不含 customDirectoryPage 展开。
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { repoRoot } from './script-utils.mjs'

const TEMPLATES_SUBDIR = path.join('node_modules', 'app-builder-lib', 'templates', 'nsis')

function findTemplatesDir() {
  const pnpmDir = path.join(repoRoot, 'node_modules', '.pnpm')
  if (!existsSync(pnpmDir)) {
    throw new Error(`app-builder-lib not installed (missing ${pnpmDir}); run pnpm install first.`)
  }
  for (const entry of readdirSync(pnpmDir)) {
    if (!entry.startsWith('app-builder-lib@')) continue
    const candidate = path.join(pnpmDir, entry, TEMPLATES_SUBDIR)
    if (existsSync(candidate)) return candidate
  }
  throw new Error(`app-builder-lib NSIS templates not found under ${pnpmDir}`)
}

// 模板与补丁文本统一为 LF 后再比较/替换，避免 CRLF 让补丁静默失效。
function normalize(text) {
  return text.replace(/\r\n/g, '\n')
}

function applyPatch(dir, { file, oldText, newText, label }) {
  const target = path.join(dir, file)
  if (!existsSync(target)) {
    throw new Error(`NSIS patch target missing: ${target} (${label})`)
  }
  const text = normalize(readFileSync(target, 'utf8'))
  const old = normalize(oldText)
  const next = normalize(newText)

  if (text.includes(next)) {
    console.log(`ok    ${file}: already patched (${label})`)
    return
  }
  if (!text.includes(old)) {
    throw new Error(`NSIS patch text not found in ${file} (${label}); template changed?`)
  }
  writeFileSync(target, text.replace(old, next), 'utf8')
  console.log(`patch ${file}: ${label}`)
}

// electron-builder 原生模板里 allowToChangeInstallationDirectory 分支的目录页代码。
const stockDirectoryPage = [
  '    !include StrContains.nsh',
  '',
  '    !insertmacro skipPageIfUpdated',
  '    !insertmacro MUI_PAGE_DIRECTORY',
  '',
  "    # pageDirectory leave doesn't work (it seems because $INSTDIR is set after custom leave function)",
  '    # so, we use instfiles pre',
  '    !define MUI_PAGE_CUSTOMFUNCTION_PRE instFilesPre',
  '',
  '    # sanitize the MUI_PAGE_DIRECTORY result to make sure it has a application name sub-folder',
  '    Function instFilesPre',
  '      ${StrContains} $0 "${APP_FILENAME}" $INSTDIR',
  '      ${If} $0 == ""',
  '        StrCpy $INSTDIR "$INSTDIR\\${APP_FILENAME}"',
  '      ${endIf}',
  '    FunctionEnd',
].join('\n')

// 改成优先展开 customDirectoryPage；否则保留原生页面。
// StrContains.nsh 只有原生页面（instFilesPre）用到，放进 !else 分支可避免
// 自定义目录页生效时出现 6010 "StrContains not referenced" 警告。
const customDirectoryPagePatch = [
  '    # XuanYu patch: use the customDirectoryPage when defined, else the stock page.',
  '    # StrContains.nsh is pulled in only by the stock page (instFilesPre uses it);',
  '    # keeping it inside !else avoids a 6010 "StrContains not referenced" warning',
  '    # when the custom directory page is active.',
  '    !ifmacrodef customDirectoryPage',
  '      !insertmacro customDirectoryPage',
  '    !else',
  '      !include StrContains.nsh',
  '      !insertmacro skipPageIfUpdated',
  '      !insertmacro MUI_PAGE_DIRECTORY',
  '',
  "      # pageDirectory leave doesn't work (it seems because $INSTDIR is set after custom leave function)",
  '      # so, we use instfiles pre',
  '      !define MUI_PAGE_CUSTOMFUNCTION_PRE instFilesPre',
  '',
  '      # sanitize the MUI_PAGE_DIRECTORY result to make sure it has a application name sub-folder',
  '      Function instFilesPre',
  '        ${StrContains} $0 "${APP_FILENAME}" $INSTDIR',
  '        ${If} $0 == ""',
  '          StrCpy $INSTDIR "$INSTDIR\\${APP_FILENAME}"',
  '        ${endIf}',
  '      FunctionEnd',
  '    !endif',
].join('\n')

const templatesDir = findTemplatesDir()

applyPatch(templatesDir, {
  file: 'assistedInstaller.nsh',
  oldText: stockDirectoryPage,
  newText: customDirectoryPagePatch,
  label: 'customDirectoryPage conditional block',
})

console.log('NSIS template patches applied.')
