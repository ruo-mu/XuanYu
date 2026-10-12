// Cross-platform entry point for the NSIS template patches.
//
// The patches live in patch-nsis-templates.ps1 because they only matter on
// Windows: electron-builder produces NSIS installers on Windows only. This
// wrapper keeps the "postinstall" hook of apps/desktop portable, so a plain
// `pnpm install` on Linux/macOS (e.g. the CI typecheck job) is a no-op instead
// of failing with `sh: 1: powershell: not found`.
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

if (process.platform !== 'win32') {
  process.exit(0)
}

const script = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  'patch-nsis-templates.ps1',
)

const result = spawnSync(
  'powershell',
  ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script],
  { stdio: 'inherit' },
)

if (result.error) {
  console.error(`NSIS template patch step failed: ${result.error.message}`)
  process.exit(1)
}

process.exit(result.status ?? 0)