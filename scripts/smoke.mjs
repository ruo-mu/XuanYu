#!/usr/bin/env node
import { existsSync, realpathSync, rmSync } from 'node:fs'
import path from 'node:path'
import { loadEnv, repoRoot, run } from './script-utils.mjs'

loadEnv()

const args = process.argv.slice(2)
const hasFlag = (name) => args.includes(name)

function option(name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`))
  if (inline) return inline.slice(name.length + 1)
  const index = args.indexOf(name)
  return index !== -1 ? args[index + 1] : undefined
}

const skipFrontend = hasFlag('--skip-frontend')
const skipRust = hasFlag('--skip-rust')
const skipBuild = hasFlag('--skip-build')
const fullRust = hasFlag('--full-rust')
const requireClean = hasFlag('--require-clean')
const noCleanRootData = hasFlag('--no-clean-root-data')
const toolchain = option('--toolchain') ?? ''

function cargoArgs(...rest) {
  return toolchain ? [`+${toolchain}`, ...rest] : rest
}

function invoke(command, commandLineArgs) {
  const result = run(command, commandLineArgs, { stdio: 'inherit', cwd: repoRoot })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}

function requireCommand(command) {
  const result = run(command, ['--version'], { stdio: 'ignore', cwd: repoRoot })
  if (result.error || result.status !== 0) {
    throw new Error(`Required command '${command}' was not found on PATH.`)
  }
}

function step(name, body) {
  console.log('')
  console.log(`==> ${name}`)
  const startedAt = Date.now()
  body()
  console.log(`<== ${name} (${((Date.now() - startedAt) / 1000).toFixed(1)}s)`)
}

function clearRootDataDirectory() {
  if (noCleanRootData) return

  const expected = path.join(repoRoot, '.xuanyu')
  if (!existsSync(expected)) return

  const resolved = realpathSync(expected)
  if (resolved !== path.join(realpathSync(repoRoot), '.xuanyu')) {
    throw new Error(`Refusing to remove unexpected path: ${resolved}`)
  }

  console.log(`Removing test data directory ${resolved}`)
  rmSync(resolved, { recursive: true, force: true })
}

step('toolchain availability', () => {
  for (const command of ['git', 'pnpm', 'node', 'cargo']) requireCommand(command)
  invoke('git', ['--version'])
  invoke('node', ['--version'])
  invoke('pnpm', ['--version'])
  invoke('cargo', cargoArgs('--version'))
})

step('workspace status', () => {
  const result = run('git', ['status', '--porcelain'], { cwd: repoRoot, encoding: 'utf8' })
  if (result.error) throw result.error
  const status = (result.stdout ?? '').trim()
  if (status) {
    console.log(status)
    if (requireClean) throw new Error('Workspace has uncommitted changes.')
  } else {
    console.log('clean')
  }
})

clearRootDataDirectory()

if (!skipFrontend) {
  step('frontend typecheck', () => {
    invoke('pnpm', ['run', 'typecheck', '--', '--pretty', 'false'])
  })

  if (!skipBuild) {
    step('frontend production build', () => {
      invoke('pnpm', ['run', 'build'])
    })
  }
}

if (!skipRust) {
  const packages = fullRust
    ? [
        'xuanyu-protocol',
        'xuanyu-extensions',
        'xuanyu-eventbus',
        'xuanyu-storage',
        'xuanyu-governance',
        'xuanyu-session',
        'xuanyu-execution',
        'xuanyu-memory',
        'xuanyu-agents',
        'xuanyu-agent-bridge',
        'xuanyu-tools',
        'xuanyu-channels',
        'xuanyu-cli',
        'xuanyu-desktop-host',
        'xuanyu-core',
      ]
    : ['xuanyu-core', 'xuanyu-execution', 'xuanyu-eventbus', 'xuanyu-storage', 'xuanyu-agent-bridge']

  for (const packageName of packages) {
    step(`cargo test -p ${packageName}`, () => {
      invoke('cargo', cargoArgs('test', '-p', packageName))
    })
  }

  step('headless smoke turn', () => {
    const smokeData = path.join(repoRoot, '.xuanyu-smoke')
    rmSync(smokeData, { recursive: true, force: true })
    invoke('cargo', cargoArgs('run', '-p', 'xuanyu-cli', '--bin', 'xuanyu', '--', '--data-dir', smokeData, 'smoke-turn'))
  })
}

clearRootDataDirectory()

console.log('')
console.log('Smoke checks completed.')
