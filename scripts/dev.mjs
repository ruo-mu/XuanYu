#!/usr/bin/env node
import path from 'node:path'
import { loadEnv, repoRoot, run } from './script-utils.mjs'

loadEnv()

const args = process.argv.slice(2)

function option(name, fallback) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`))
  if (inline) return inline.slice(name.length + 1)
  const index = args.indexOf(name)
  return index !== -1 && args[index + 1] !== undefined ? args[index + 1] : fallback
}

process.env.HOST = option('--host', '127.0.0.1')
process.env.PORT = option('--port', '5173')

// 引擎按自身所在目录解析同级 pg/；dev 引擎跑在 target/debug，把便携 PostgreSQL 同步落到那里。
const postgres = run(
  process.execPath,
  [path.join(repoRoot, 'scripts', 'fetch-postgres.mjs'), '--profile=debug'],
  { stdio: 'inherit', cwd: repoRoot },
)
if (postgres.error) throw postgres.error
if (postgres.status !== 0) process.exit(postgres.status ?? 1)

const result = run('pnpm', ['run', 'dev'], { stdio: 'inherit', cwd: repoRoot })
if (result.error) throw result.error
process.exit(result.status ?? 0)
