#!/usr/bin/env node
// 构建期获取内置便携 PostgreSQL 与 pgvector，落到引擎可执行文件同级的 pg/ 目录。
// Windows 取 EDB 官方 binaries-only 归档；macOS / Linux 取 conda-forge 可重定位构建，
// 并按 repodata 的 depends 递归拉取完整运行时依赖闭包（libicu / openssl / zlib 等）；
// 平台 repodata 优先，缺名时回退 noarch repodata（时区数据 tzdata 即来自 noarch），
// 使 pg/ 自包含、无需机器上预装 conda。
// conda 的 .conda 归档是 ZIP 容器（内含 zstd 压缩的 tar），这里用 node:zlib 原生解析，
// 不依赖系统 tar 是否支持 zip / zstd；Windows .zip 与 Unix .tar.bz2 仍交给系统 tar。
//
// 落位：
//   默认            -> target/engine/<arch>/pg   打包用，electron-builder extraResources 读取
//   --profile=<p>   -> target/<p>/pg             dev 用，与 cargo debug/release 产物同级
//   --arch=<arch>   显式指定目标架构，缺省按 CARGO_BUILD_TARGET / 本机架构解析
// 归档缓存在 target/pg-cache/（文件名含来源 sha256），命中即复用并校验；pg 目录齐备时整体跳过。
import { createHash } from 'node:crypto'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  readlinkSync,
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import path from 'node:path'
import { inflateRawSync, zstdDecompressSync } from 'node:zlib'
import { loadEnv, repoRoot, run } from './script-utils.mjs'

loadEnv()

const ARCH_BY_TARGET = {
  'x86_64-pc-windows-msvc': 'x64',
  'x86_64-apple-darwin': 'x64',
  'aarch64-apple-darwin': 'arm64',
  'x86_64-unknown-linux-gnu': 'x64',
}

const POSTGRES_WINDOWS = {
  url: 'https://get.enterprisedb.com/postgresql/postgresql-17.6-1-windows-x64-binaries.zip',
  sha256: 'd378882abd001a186735acd6f6ba716bca6ccd192e800412d4fd15ed25376b3e',
}
const PGVECTOR_WINDOWS = {
  url: 'https://github.com/andreiramani/pgvector_pgsql_windows/releases/download/0.8.1_17.6/vector.v0.8.1-pg17.zip',
  sha256: '0ccf5dafc5a98c290871244aa71f3189afc6a441c67cc12eca938473faa3132e',
}

const CONDA_CHANNEL = 'https://conda.anaconda.org/conda-forge'

// 运行期只保留这四个程序加可选的 pg_isready；共享库（含 Windows bin 下的依赖 DLL）不能删。
const KEEP_BINARIES = new Set(['postgres', 'initdb', 'pg_ctl', 'psql', 'pg_isready'])
const SHARED_LIBRARY = /\.(dll|so|so\.\d+(\.\d+)*|dylib)$/i
const STRIP_DIRS = new Set([
  'include',
  'doc',
  'man',
  'symbols',
  'info',
  'conda-meta',
  'pgAdmin',
  'pgAdmin 4',
  'StackBuilder',
  'stackbuilder',
])
const STRIP_FILES = /\.(lib|pdb)$/i

const args = process.argv.slice(2)

function option(name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`))
  if (inline) return inline.slice(name.length + 1)
  const index = args.indexOf(name)
  return index !== -1 ? args[index + 1] : undefined
}

const profile = option('--profile')
if (profile && profile !== 'debug' && profile !== 'release') {
  throw new Error(`Invalid --profile '${profile}' (expected debug or release)`)
}

const rustTarget = process.env.CARGO_BUILD_TARGET?.trim() || null
const engineArch = option('--arch') ?? (rustTarget ? ARCH_BY_TARGET[rustTarget] : process.arch)
if (!engineArch) {
  throw new Error(`Unknown rust target for PostgreSQL staging: ${rustTarget}`)
}

const outDir = profile
  ? path.join(repoRoot, 'target', profile, 'pg')
  : path.join(repoRoot, 'target', 'engine', engineArch, 'pg')

async function stageWindows() {
  const sources = [
    { label: 'PostgreSQL (EDB)', ...POSTGRES_WINDOWS },
    { label: 'pgvector', ...PGVECTOR_WINDOWS },
  ]
  const postgresArchive = await fetchArchive(POSTGRES_WINDOWS.url, POSTGRES_WINDOWS.sha256)
  const vectorArchive = await fetchArchive(PGVECTOR_WINDOWS.url, PGVECTOR_WINDOWS.sha256)

  const work = cleanWorkDir()
  const extractDir = path.join(work, 'postgres')
  extractArchiveTo(postgresArchive, extractDir)
  const nested = path.join(extractDir, 'pgsql')
  if (!existsSync(nested)) {
    throw new Error(`Unexpected PostgreSQL archive layout (missing pgsql/): ${POSTGRES_WINDOWS.url}`)
  }
  const pgDir = path.join(work, 'pg')
  renameSync(nested, pgDir)

  const vectorDir = path.join(work, 'vector')
  extractArchiveTo(vectorArchive, vectorDir)
  mergeTree(vectorDir, pgDir)

  finalize(pgDir)
  rmSync(work, { recursive: true, force: true })
  return {
    sources,
    packages: [
      { name: 'postgresql', version: '17.6' },
      { name: 'pgvector', version: '0.8.1' },
    ],
  }
}

async function stageUnix() {
  const subdir = condaSubdir(engineArch)
  const resolved = await resolveCondaSources(subdir)
  const sources = [...resolved.roots.values()].map((pkg) => ({
    label: `${pkg.name} (conda-forge ${pkg.subdir})`,
    url: `${CONDA_CHANNEL}/${pkg.subdir}/${pkg.filename}`,
    sha256: pkg.sha256,
  }))

  const work = cleanWorkDir()
  const pgDir = path.join(work, 'pg')
  mkdirSync(pgDir, { recursive: true })

  // 依赖闭包先合并，roots（postgresql / pgvector）最后合并，保证其同名文件不被依赖覆盖。
  const mergeOrder = [
    ...resolved.packages.filter((pkg) => !resolved.roots.has(pkg.name)),
    ...resolved.roots.values(),
  ]
  for (const pkg of mergeOrder) {
    const archive = await fetchArchive(`${CONDA_CHANNEL}/${pkg.subdir}/${pkg.filename}`, pkg.sha256)
    const extractDir = path.join(work, `pkg-${pkg.name}`)
    extractArchiveTo(archive, extractDir)
    mergeTree(extractDir, pgDir)
  }

  finalize(pgDir)
  rmSync(work, { recursive: true, force: true })
  return {
    sources,
    packages: resolved.packages.map((pkg) => ({ name: pkg.name, version: pkg.version })),
  }
}

function condaSubdir(arch) {
  if (process.platform === 'darwin') return arch === 'arm64' ? 'osx-arm64' : 'osx-64'
  return arch === 'arm64' ? 'linux-aarch64' : 'linux-64'
}

function cleanWorkDir() {
  const work = path.join(repoRoot, 'target', `pg-work-${engineArch}`)
  rmSync(work, { recursive: true, force: true })
  mkdirSync(work, { recursive: true })
  return work
}

// 剥离文档与客户端工具后，把就地构建好的程序目录整体挪到落位处。
function finalize(pgDir) {
  ensurePgvector(pgDir)
  stripTree(pgDir)
  stripBin(pgDir)
  verifyPgDir(pgDir)
  rmSync(outDir, { recursive: true, force: true })
  mkdirSync(path.dirname(outDir), { recursive: true })
  renameSync(pgDir, outDir)
}

async function fetchArchive(url, sha256) {
  const cacheDir = path.join(repoRoot, 'target', 'pg-cache')
  mkdirSync(cacheDir, { recursive: true })
  const name = `${sha256.slice(0, 16)}-${path.basename(new URL(url).pathname)}`
  const cached = path.join(cacheDir, name)
  if (existsSync(cached) && sha256File(cached) === sha256) {
    console.log(`cache ${path.relative(repoRoot, cached)}`)
    return cached
  }
  console.log(`fetch ${url}`)
  let response
  try {
    response = await fetch(url)
  } catch (err) {
    throw new Error(`Cannot download ${url}: ${err.message}`)
  }
  if (!response.ok) throw new Error(`Cannot download ${url}: HTTP ${response.status}`)
  const bytes = Buffer.from(await response.arrayBuffer())
  const actual = createHash('sha256').update(bytes).digest('hex')
  if (actual !== sha256) {
    throw new Error(`sha256 mismatch for ${url}: expected ${sha256}, got ${actual}`)
  }
  writeFileSync(cached, bytes)
  return cached
}

function sha256File(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex')
}

async function fetchJson(url) {
  let response
  try {
    response = await fetch(url)
  } catch (err) {
    throw new Error(`Cannot download ${url}: ${err.message}`)
  }
  if (!response.ok) throw new Error(`Cannot download ${url}: HTTP ${response.status}`)
  return response.json()
}

async function resolveCondaSources(subdir) {
  const urls = [
    `${CONDA_CHANNEL}/${subdir}/current_repodata.json`,
    `${CONDA_CHANNEL}/${subdir}/repodata.json`,
  ]
  let lastError = null
  for (const url of urls) {
    let repodata
    try {
      repodata = await fetchJson(url)
    } catch (err) {
      lastError = err
      continue
    }
    const index = condaIndex(repodata, subdir)
    const pair = resolvePair(index)
    if (!pair) continue
    const noarch = await fetchNoarchIndex()
    const closure = resolveClosure(index, noarch, [pair.postgresql, pair.pgvector])
    return {
      roots: new Map([
        [pair.postgresql.name, pair.postgresql],
        [pair.pgvector.name, pair.pgvector],
      ]),
      packages: [...closure.values()],
    }
  }
  throw new Error(
    `Cannot resolve conda-forge postgresql + pgvector for ${subdir}` +
      (lastError ? `: ${lastError.message}` : ''),
  )
}

// tzdata 等只有 noarch 构建；平台没命中时从 noarch repodata 回退。
async function fetchNoarchIndex() {
  const urls = [
    `${CONDA_CHANNEL}/noarch/current_repodata.json`,
    `${CONDA_CHANNEL}/noarch/repodata.json`,
  ]
  let lastError = null
  for (const url of urls) {
    try {
      return condaIndex(await fetchJson(url), 'noarch')
    } catch (err) {
      lastError = err
    }
  }
  throw new Error(`Cannot load conda-forge noarch repodata${lastError ? `: ${lastError.message}` : ''}`)
}

// 同名包可能同时有 .tar.bz2 与 .conda，按包名索引为 name -> entries[]；subdir 决定下载路径。
function condaIndex(repodata, subdir) {
  const merged = { ...(repodata.packages ?? {}), ...(repodata['packages.conda'] ?? {}) }
  const index = new Map()
  for (const [filename, meta] of Object.entries(merged)) {
    const entry = { filename, subdir, ...meta }
    const list = index.get(entry.name)
    if (list) list.push(entry)
    else index.set(entry.name, [entry])
  }
  return index
}

// 选定 PostgreSQL 主版本（优先 17.x，否则取最新且 >= 15）与 ABI 匹配的 pgvector；.tar.bz2 优先于 .conda。
function resolvePair(index) {
  const postgresql = (index.get('postgresql') ?? []).filter(
    (entry) => Number(entry.version.split('.')[0]) >= 15,
  )
  const pgvector = index.get('pgvector') ?? []
  const majors = [...new Set(postgresql.map((entry) => Number(entry.version.split('.')[0])))].sort(
    (a, b) => (a === 17 ? -1 : b === 17 ? 1 : b - a),
  )
  for (const major of majors) {
    const pool = postgresql.filter((entry) => Number(entry.version.split('.')[0]) === major)
    const pgArtifact = pickArtifact(
      pool.filter((entry) => entry.version === latestEntry(pool).version),
    )
    if (!pgArtifact) continue
    const vecPool = pgvector.filter((entry) => requiredMajor(entry) === major)
    if (!vecPool.length) continue
    const vecArtifact = pickArtifact(
      vecPool.filter((entry) => entry.version === latestEntry(vecPool).version),
    )
    if (vecArtifact) return { postgresql: pgArtifact, pgvector: vecArtifact }
  }
  return null
}

// 从 repodata 的 depends 递归解析运行时依赖闭包；roots 为 postgresql / pgvector 本身。
// 平台 repodata 优先，仅当平台无此包名时才回退 noarch（noarch 无 subdir / build 变体）。
function resolveClosure(platform, noarch, roots) {
  const resolved = new Map()
  const queue = []
  for (const root of roots) {
    resolved.set(root.name, root)
    queue.push(root)
  }
  while (queue.length) {
    const entry = queue.shift()
    for (const dep of entry.depends ?? []) {
      const { name, constraint } = parseDependency(dep)
      if (!name || isVirtualDependency(name) || resolved.has(name)) continue
      const entries = platform.has(name) ? platform.get(name) : noarch.get(name)
      const artifact = selectArtifact(entries, constraint)
      if (!artifact) continue
      resolved.set(name, artifact)
      queue.push(artifact)
    }
  }
  return resolved
}

// conda 依赖串形如 "libicu >=73.1,<74.0a0" / "python_abi 3.11.* *_cp311"：取包名与版本约束，忽略 build string。
function parseDependency(dep) {
  const tokens = String(dep).trim().split(/\s+/)
  const name = tokens[0] ?? ''
  const constraint = tokens[1] && /^[<>=!~0-9]/.test(tokens[1]) ? tokens[1] : ''
  return { name, constraint }
}

// 虚拟包与元包没有可下载产物，跳过。
function isVirtualDependency(name) {
  return name.startsWith('__') || name.endsWith('_abi') || name.startsWith('m2-')
}

// pgvector 的 ABI 由依赖的 libpq / postgresql 主版本决定（conda-forge 走 libpq）。
function requiredMajor(entry) {
  for (const dep of entry.depends ?? []) {
    const { name, constraint } = parseDependency(dep)
    if (name !== 'postgresql' && name !== 'libpq') continue
    const match = constraint.match(/\d+/)
    if (match) return Number(match[0])
  }
  return null
}

function selectArtifact(entries, constraint) {
  if (!entries?.length) return null
  const matching = entries.filter(
    (entry) => !constraint || versionSatisfies(entry.version, constraint),
  )
  if (!matching.length) return null
  return pickArtifact(matching.filter((entry) => entry.version === latestEntry(matching).version))
}

// 宽松的版本区间判断：支持 >=x,<y、x.*、==x、!=x、>x、<x 与裸 x（按前缀匹配）。
function versionSatisfies(version, constraint) {
  if (!constraint) return true
  return constraint.split(',').every((clause) => clauseSatisfies(version, clause.trim()))
}

function clauseSatisfies(version, clause) {
  if (!clause) return true
  if (clause.endsWith('.*')) return versionMatchesPrefix(version, clause.slice(0, -2))
  const match = clause.match(/^(>=|<=|==|!=|>|<)(.+)$/)
  if (!match) return versionMatchesPrefix(version, clause)
  const diff = compareVersions(version, match[2])
  switch (match[1]) {
    case '>=':
      return diff >= 0
    case '<=':
      return diff <= 0
    case '>':
      return diff > 0
    case '<':
      return diff < 0
    case '==':
      return diff === 0
    case '!=':
      return diff !== 0
    default:
      return true
  }
}

function versionMatchesPrefix(version, prefix) {
  const left = String(version).split(/[._-]/)
  const right = String(prefix).split(/[._-]/)
  return right.every((token, index) => (left[index] ?? '') === token)
}

function latestEntry(entries) {
  return entries.reduce((best, entry) =>
    compareVersions(entry.version, best.version) > 0 ? entry : best,
  )
}

function pickArtifact(entries) {
  return (
    entries.find((entry) => entry.filename.endsWith('.tar.bz2')) ??
    entries.find((entry) => entry.filename.endsWith('.conda'))
  )
}

function compareVersions(a, b) {
  const left = String(a).split(/[._-]/)
  const right = String(b).split(/[._-]/)
  for (let i = 0; i < Math.max(left.length, right.length); i += 1) {
    const la = Number(left[i] ?? 0)
    const ra = Number(right[i] ?? 0)
    if (Number.isNaN(la) || Number.isNaN(ra)) {
      const sa = left[i] ?? ''
      const sb = right[i] ?? ''
      if (sa !== sb) return sa < sb ? -1 : 1
    } else if (la !== ra) {
      return la - ra
    }
  }
  return 0
}

function extractArchiveTo(archive, destDir) {
  mkdirSync(destDir, { recursive: true })
  const lower = archive.toLowerCase()
  if (lower.endsWith('.conda')) {
    extractConda(archive, destDir)
    return
  }
  if (lower.endsWith('.tar.bz2') || lower.endsWith('.tbz2')) {
    runTar(['-xjf', archive, '-C', destDir])
    return
  }
  runTar(['-xf', archive, '-C', destDir])
}

// .conda 是 ZIP 容器，内含 pkg-*.tar.zst 与 info-*.tar.zst；仅取 pkg 那份并原生解包。
function extractConda(archive, destDir) {
  const entry = readZipEntry(
    readFileSync(archive),
    (name) => name.startsWith('pkg-') && name.endsWith('.tar.zst'),
  )
  if (!entry) {
    throw new Error(`Unexpected .conda layout in ${path.basename(archive)} (no pkg-*.tar.zst)`)
  }
  extractTarTo(zstdDecompressSync(entry.data), destDir)
}

// 极简 ZIP 读取：定位中央目录，返回首个满足 predicate 的条目名与解压后内容。
function readZipEntry(buffer, predicate) {
  const eocd = findEndOfCentralDirectory(buffer)
  const count = buffer.readUInt16LE(eocd + 10)
  let offset = buffer.readUInt32LE(eocd + 16)
  for (let index = 0; index < count; index += 1) {
    if (buffer.readUInt32LE(offset) !== 0x02014b50) {
      throw new Error('Corrupt ZIP central directory')
    }
    const method = buffer.readUInt16LE(offset + 10)
    const compressedSize = buffer.readUInt32LE(offset + 20)
    const nameLength = buffer.readUInt16LE(offset + 28)
    const extraLength = buffer.readUInt16LE(offset + 30)
    const commentLength = buffer.readUInt16LE(offset + 32)
    const localOffset = buffer.readUInt32LE(offset + 42)
    const name = buffer.toString('utf8', offset + 46, offset + 46 + nameLength)
    if (predicate(name)) {
      return { name, data: readZipEntryData(buffer, localOffset, method, compressedSize) }
    }
    offset += 46 + nameLength + extraLength + commentLength
  }
  return null
}

function readZipEntryData(buffer, localOffset, method, compressedSize) {
  if (buffer.readUInt32LE(localOffset) !== 0x04034b50) {
    throw new Error('Corrupt ZIP local header')
  }
  const nameLength = buffer.readUInt16LE(localOffset + 26)
  const extraLength = buffer.readUInt16LE(localOffset + 28)
  const start = localOffset + 30 + nameLength + extraLength
  const compressed = buffer.subarray(start, start + compressedSize)
  if (method === 0) return Buffer.from(compressed)
  if (method === 8) return inflateRawSync(compressed)
  throw new Error(`Unsupported ZIP compression method ${method}`)
}

function findEndOfCentralDirectory(buffer) {
  const floor = Math.max(0, buffer.length - 22 - 0xffff)
  for (let offset = buffer.length - 22; offset >= floor; offset -= 1) {
    if (buffer.readUInt32LE(offset) === 0x06054b50) return offset
  }
  throw new Error('Cannot locate ZIP end-of-central-directory record')
}

// 极简 USTAR/PAX tar 解包：覆盖普通文件 / 目录 / 符号链接 / PAX 与 GNU 长名扩展头。
function extractTarTo(buffer, destDir) {
  mkdirSync(destDir, { recursive: true })
  const resolvedDest = path.resolve(destDir)
  let offset = 0
  let longName = null
  let longLink = null
  let pax = null
  let globalPax = null
  while (offset + 512 <= buffer.length) {
    const header = buffer.subarray(offset, offset + 512)
    if (header.every((byte) => byte === 0)) break
    const size = readTarNumber(header, 124, 12)
    const mode = readTarNumber(header, 100, 8) & 0o777
    const typeflag = String.fromCharCode(header[156])
    const dataStart = offset + 512
    const dataEnd = dataStart + size

    if (typeflag === 'x' || typeflag === 'g') {
      const records = parsePaxRecords(buffer.toString('utf8', dataStart, dataEnd))
      if (typeflag === 'g') globalPax = { ...(globalPax ?? {}), ...records }
      else pax = records
      offset = dataStart + tarPadding(size)
      continue
    }
    if (typeflag === 'L' || typeflag === 'K') {
      const value = buffer.toString('utf8', dataStart, dataEnd).replace(/\0+$/, '')
      if (typeflag === 'L') longName = value
      else longLink = value
      offset = dataStart + tarPadding(size)
      continue
    }

    const extended = { ...(globalPax ?? {}), ...(pax ?? {}) }
    const prefix = readTarString(header, 345, 155)
    const rawName = readTarString(header, 0, 100)
    const name = (extended.path ?? longName ?? (prefix ? `${prefix}/${rawName}` : rawName)).replace(
      /^\.\//,
      '',
    )
    const link = extended.linkpath ?? longLink ?? readTarString(header, 157, 100)
    const contentSize = extended.size != null ? Number(extended.size) : size
    pax = null
    longName = null
    longLink = null

    const full = path.resolve(destDir, name)
    if (full !== resolvedDest && !full.startsWith(resolvedDest + path.sep)) {
      throw new Error(`Refusing unsafe tar entry path: ${name}`)
    }
    if (typeflag === '5') {
      mkdirSync(full, { recursive: true })
      chmodSync(full, mode || 0o755)
    } else if (typeflag === '2') {
      mkdirSync(path.dirname(full), { recursive: true })
      rmSync(full, { force: true })
      try {
        symlinkSync(link, full)
      } catch {
        // 少数平台不允许创建符号链接；conda 的 Unix 包才带符号链接，忽略即可。
      }
    } else if (typeflag === '0' || typeflag === '\0' || typeflag === '7') {
      mkdirSync(path.dirname(full), { recursive: true })
      const fileMode = name.startsWith('bin/') ? 0o755 : mode || 0o644
      writeFileSync(full, buffer.subarray(dataStart, dataStart + contentSize), { mode: fileMode })
      chmodSync(full, fileMode)
    }
    offset = dataStart + tarPadding(size)
  }
}

function tarPadding(size) {
  return Math.ceil(size / 512) * 512
}

function readTarString(block, offset, length) {
  let end = offset
  while (end < offset + length && block[end] !== 0) end += 1
  return block.toString('utf8', offset, end)
}

function readTarNumber(block, offset, length) {
  if (block[offset] & 0x80) {
    let value = block[offset] & 0x7f
    for (let index = 1; index < length; index += 1) value = value * 256 + block[offset + index]
    return value
  }
  const text = readTarString(block, offset, length).trim()
  return text ? Number.parseInt(text, 8) : 0
}

function parsePaxRecords(text) {
  const records = {}
  let offset = 0
  while (offset < text.length) {
    const space = text.indexOf(' ', offset)
    if (space === -1) break
    const length = Number.parseInt(text.slice(offset, space), 10)
    if (!Number.isFinite(length) || length <= 0 || offset + length > text.length) break
    const record = text.slice(offset, offset + length)
    const separator = record.indexOf('=')
    if (separator !== -1) {
      const key = record.slice(space - offset + 1, separator)
      records[key] = record.slice(separator + 1).replace(/\n$/, '')
    }
    offset += length
  }
  return records
}

function runTar(tarArgs) {
  const result = run('tar', tarArgs, { stdio: 'inherit', cwd: repoRoot })
  if (result.error) throw new Error(`Failed to run tar: ${result.error.message}`)
  if (result.status !== 0) {
    throw new Error(
      `tar failed (status ${result.status}) running: tar ${tarArgs.join(' ')}. ` +
        'The platform tar cannot extract this archive format.',
    )
  }
}

function mergeTree(sourceDir, targetDir) {
  for (const entry of readdirSync(sourceDir, { withFileTypes: true })) {
    const from = path.join(sourceDir, entry.name)
    const to = path.join(targetDir, entry.name)
    if (entry.isDirectory()) {
      mkdirSync(to, { recursive: true })
      mergeTree(from, to)
    } else if (entry.isSymbolicLink()) {
      rmSync(to, { force: true })
      symlinkSync(readlinkSync(from), to)
    } else if (entry.isFile()) {
      mkdirSync(path.dirname(to), { recursive: true })
      copyFileSync(from, to)
    }
  }
}

// conda-forge 的 pgvector 把扩展放在 share/postgresql/extension/，这里同时铺一份到
// share/extension/，并确保 lib/vector.{so,dylib,dll} 就位。
function ensurePgvector(pgDir) {
  const extDir = path.join(pgDir, 'share', 'extension')
  mkdirSync(extDir, { recursive: true })
  const files = findFiles(
    pgDir,
    (name) => name === 'vector.control' || /^vector--.*\.sql$/.test(name),
  )
  if (!files.some((file) => path.basename(file) === 'vector.control')) {
    throw new Error('pgvector extension files (vector.control / vector--*.sql) not found')
  }
  for (const file of files) {
    const target = path.join(extDir, path.basename(file))
    if (path.resolve(file) !== path.resolve(target)) copyFileSync(file, target)
  }
  const library = findFiles(pgDir, (name) => /^vector\.(so|dylib|dll)$/.test(name))[0]
  if (library) {
    const target = path.join(pgDir, 'lib', path.basename(library))
    mkdirSync(path.dirname(target), { recursive: true })
    if (path.resolve(library) !== path.resolve(target)) copyFileSync(library, target)
  }
}

function findFiles(root, predicate) {
  const found = []
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.isFile() && predicate(entry.name)) found.push(full)
    }
  }
  if (existsSync(root)) walk(root)
  return found
}

function stripTree(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      if (STRIP_DIRS.has(entry.name)) {
        rmSync(full, { recursive: true, force: true })
        continue
      }
      stripTree(full)
    } else if (entry.isFile() && STRIP_FILES.test(entry.name)) {
      rmSync(full, { force: true })
    }
  }
}

function stripBin(pgDir) {
  const binDir = path.join(pgDir, 'bin')
  if (!existsSync(binDir)) return
  for (const entry of readdirSync(binDir, { withFileTypes: true })) {
    const full = path.join(binDir, entry.name)
    if (entry.isDirectory()) {
      rmSync(full, { recursive: true, force: true })
      continue
    }
    const base = entry.name.toLowerCase().endsWith('.exe') ? entry.name.slice(0, -4) : entry.name
    if (KEEP_BINARIES.has(base) || SHARED_LIBRARY.test(entry.name)) continue
    rmSync(full, { force: true })
  }
}

function verifyPgDir(dir) {
  const suffix = process.platform === 'win32' ? '.exe' : ''
  for (const name of ['postgres', 'initdb', 'pg_ctl', 'psql']) {
    if (!existsSync(path.join(dir, 'bin', `${name}${suffix}`))) {
      throw new Error(`PostgreSQL program directory is incomplete: missing bin/${name}${suffix}`)
    }
  }
  if (!existsSync(path.join(dir, 'lib')) || !existsSync(path.join(dir, 'share'))) {
    throw new Error('PostgreSQL program directory is incomplete: missing lib/ or share/')
  }
  // 时区数据必须存在：EDB 归档自带 share/timezone；conda-forge 的 postgresql 不含时区，
  // 其 postgres 二进制的 TZDIR 指向 tzdata 包的 share/zoneinfo。两者皆无即视为不完整。
  if (
    !existsSync(path.join(dir, 'share', 'timezone')) &&
    !existsSync(path.join(dir, 'share', 'zoneinfo'))
  ) {
    throw new Error('PostgreSQL program directory is incomplete: missing share/timezone or share/zoneinfo')
  }
  if (!existsSync(path.join(dir, 'share', 'extension', 'vector.control'))) {
    throw new Error('PostgreSQL program directory is incomplete: missing share/extension/vector.control')
  }
}

function isPgDirComplete(dir) {
  try {
    verifyPgDir(dir)
    return true
  } catch {
    return false
  }
}

function directorySize(dir) {
  let total = 0
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name)
      if (entry.isDirectory()) walk(full)
      else if (entry.isFile()) total += statSync(full).size
    }
  }
  walk(dir)
  return total
}

function countSharedLibraries(libDir) {
  return findFiles(libDir, (name) => SHARED_LIBRARY.test(name)).length
}

try {
  if (isPgDirComplete(outDir)) {
    console.log(`PostgreSQL already staged: ${path.relative(repoRoot, outDir)}`)
  } else {
    const staged = process.platform === 'win32' ? await stageWindows() : await stageUnix()
    const size = directorySize(outDir) / (1024 * 1024)
    const libraries = countSharedLibraries(path.join(outDir, 'lib'))
    console.log('')
    console.log(
      `PostgreSQL staged: ${path.relative(repoRoot, outDir)} ` +
        `(${size.toFixed(1)} MB, ${staged.packages.length} packages, ${libraries} shared libraries)`,
    )
    console.log(`  packages: ${staged.packages.map((pkg) => `${pkg.name} ${pkg.version}`).join(', ')}`)
    for (const source of staged.sources) {
      console.log(`  ${source.label}: ${source.url}`)
      console.log(`    sha256 ${source.sha256}`)
    }
    if (process.platform !== 'win32' && staged.packages.length <= 2) {
      console.warn('  warning: no conda dependency packages resolved; pg/ may not be self-contained')
    }
  }
} catch (err) {
  console.error(`fetch-postgres: ${err.message}`)
  process.exit(1)
}
