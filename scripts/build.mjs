#!/usr/bin/env node
import { loadEnv, repoRoot, run } from './script-utils.mjs'

loadEnv()

const result = run('pnpm', ['run', 'build'], { stdio: 'inherit', cwd: repoRoot })
if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)

console.log('')
console.log('Dev build ready. For release package:')
console.log('  pnpm dist')
