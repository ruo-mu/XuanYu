#!/usr/bin/env node
// 按目标架构编译引擎二进制，并放到 electron-builder 的 extraResources 约定位置：
// target/engine/<electronArch>/<binary>。CARGO_BUILD_TARGET 决定交叉编译目标，
// 未设置时使用本机架构。
import { copyFileSync, mkdirSync } from 'node:fs'
import path from 'node:path'
import { loadEnv, repoRoot, run } from './script-utils.mjs'

loadEnv()

const ARCH_BY_TARGET = {
  'x86_64-pc-windows-msvc': 'x64',
  'x86_64-apple-darwin': 'x64',
  'aarch64-apple-darwin': 'arm64',
  'x86_64-unknown-linux-gnu': 'x64',
}

const rustTarget = process.env.CARGO_BUILD_TARGET?.trim() || null
const engineArch = rustTarget ? ARCH_BY_TARGET[rustTarget] : process.arch
if (!engineArch) {
  throw new Error(`Unknown rust target for engine staging: ${rustTarget}`)
}

const binaryName = process.platform === 'win32' ? 'xuanyu-engine-server.exe' : 'xuanyu-engine-server'

const build = run('cargo', ['build', '-p', 'xuanyu-cli', '--bins', '--release'], {
  stdio: 'inherit',
  cwd: repoRoot,
})
if (build.error) throw build.error
if (build.status !== 0) process.exit(build.status ?? 1)

const source = path.join(repoRoot, 'target', rustTarget ?? '', 'release', binaryName)
const destinationDir = path.join(repoRoot, 'target', 'engine', engineArch)
mkdirSync(destinationDir, { recursive: true })
const destination = path.join(destinationDir, binaryName)
copyFileSync(source, destination)

console.log(`Engine staged for ${engineArch}: ${path.relative(repoRoot, destination)}`)

// 引擎按自身所在目录解析同级 pg/，因此便携 PostgreSQL 与引擎二进制落同一目标架构目录。
const postgres = run(process.execPath, [path.join(repoRoot, 'scripts', 'fetch-postgres.mjs'), `--arch=${engineArch}`], {
  stdio: 'inherit',
  cwd: repoRoot,
})
if (postgres.error) throw postgres.error
if (postgres.status !== 0) process.exit(postgres.status ?? 1)
