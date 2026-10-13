import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const isWindows = process.platform === 'win32'

// 本地环境变量统一放在仓库根 .env（见根 .gitignore），存在才加载。
export function loadEnv() {
  const envFile = path.join(repoRoot, '.env')
  if (!existsSync(envFile)) return
  for (const line of readFileSync(envFile, 'utf8').split(/\r?\n/)) {
    const entry = line.trim()
    if (!entry || entry.startsWith('#')) continue
    const separator = entry.indexOf('=')
    if (separator <= 0) continue
    const key = entry.slice(0, separator).trim()
    let value = entry.slice(separator + 1).trim()
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1)
    }
    if (key && process.env[key] === undefined) process.env[key] = value
  }
}

function shellQuote(value) {
  return /[\s"]/.test(value) ? `"${value.replace(/"/g, '\\"')}"` : value
}

// Windows 下 pnpm 等入口是 .cmd 包装，必须交给 cmd.exe 解析才能启动。
export function run(command, args = [], options = {}) {
  if (isWindows) {
    const line = [command, ...args.map(shellQuote)].join(' ')
    return spawnSync(line, { shell: true, ...options })
  }
  return spawnSync(command, args, options)
}
