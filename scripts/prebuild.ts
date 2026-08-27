import type { GitHubSource } from './preset-source'
/**
 * prebuild：把 `src-tauri/resources/preset-plugins.json` 中标记 `internal: true`
 * 的插件制备为随包产物，拷入 `src-tauri/resources/preset-plugins/<id>/`
 * （随 `bundle.resources` 随安装包分发）。两种来源：
 *
 * - `github:owner/repo[#<full-commit-sha>]`：从上游仓库克隆、安装依赖并构建
 *   （源码形态的插件；指定 SHA 时按该不可变版本检出）；
 * - npm 包名（`name[@version]`）：从 npm registry 拉取已发布产物，跳过构建
 *   （发布包自带 lib/，如 dsh-tauri@0.2.0）。
 *
 * 由 `pnpm build` 的 prebuild 生命周期自动触发（tauri 的 `beforeBuildCommand` 为
 * `pnpm build`，pnpm 先执行 `prebuild` 脚本）。应用启动时（service::plugin::internal）
 * 会核对内置插件是否已安装、安装路径是否仍指向该捆绑目录，未满足即强制重装。
 *
 * 约束：仅用 Node 内置模块（零新增依赖）；需要 git 与 pnpm 在 PATH 上；
 * 构建机器需可访问 GitHub 与 npm registry。通过 `tsx scripts/prebuild.ts`
 * 直接运行（TS + ESM），无需预编译。
 */
import { Buffer } from 'node:buffer'
import { spawnSync } from 'node:child_process'
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import process from 'node:process'
import { pathToFileURL } from 'node:url'
import {
  matchesSha512Integrity,
  parseGitHubSource,
  parseNpmSource,
  parseSha512Integrity,
} from './preset-source'

interface PresetPlugin {
  id: string
  spec: string
  internal?: boolean
  integrity?: string
}

const REPO_ROOT = resolve(import.meta.dirname, '..')
const PRESET_FILE = join(REPO_ROOT, 'src-tauri', 'resources', 'preset-plugins.json')
const BUNDLE_ROOT = join(REPO_ROOT, 'src-tauri', 'resources', 'preset-plugins')

function die(message: string): never {
  console.error(`[prebuild] ${message}`)
  process.exit(1)
}

/** 同步执行命令，非零退出码即终止构建（内置插件缺失是发布缺陷，必须响亮失败）。 */
function run(program: string, args: readonly string[], cwd: string): void {
  console.log(`[prebuild] $ ${program} ${args.join(' ')}`)
  const result = spawnSync(program, [...args], {
    cwd,
    stdio: 'inherit',
    shell: process.platform === 'win32',
  })
  if (result.error !== undefined) {
    die(`${program} 启动失败: ${result.error.message}`)
  }
  if (result.status !== 0) {
    die(`${program} ${args.join(' ')} 退出码 ${result.status}`)
  }
}

/** 克隆 GitHub 来源；指定 revision 时按完整 SHA 取对象并脱离分支检出，避免构建漂移。 */
function cloneGitHubSource(preset: PresetPlugin, temp: string): string {
  let source: GitHubSource
  try {
    source = parseGitHubSource(preset.spec)
  }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    die(`${preset.id}: ${message}`)
  }

  const clone = join(temp, preset.id)
  // `git clone --branch <sha>` 不支持任意提交 SHA；初始化空仓库后直接 fetch 该
  // 对象既能保留浅克隆，也能确保最终检出的是清单固定的不可变版本。
  run('git', ['init', clone], temp)
  run('git', ['-C', clone, 'remote', 'add', 'origin', source.remoteUrl], temp)
  run('git', ['-C', clone, 'fetch', '--depth', '1', 'origin', source.revision], temp)
  run('git', ['-C', clone, 'checkout', '--detach', '--quiet', 'FETCH_HEAD'], temp)
  return clone
}

/**
 * 下载内置 npm tarball 并先验证固定 SHA-512 SRI。只接受 registry.npmjs.org 的
 * 精确版本 URL，且禁用重定向，避免依赖 npm 元数据与安装器的第二次下载之间出现
 * 内容漂移；只有验证后的本地 tarball 会交给 pnpm 安装。
 */
async function downloadVerifiedNpmTarball(
  preset: PresetPlugin,
  tarballUrl: string,
  integrity: string,
  temp: string,
): Promise<string> {
  let response: Response
  try {
    response = await fetch(tarballUrl, { redirect: 'error' })
  }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    die(`${preset.id}: npm tarball 下载失败: ${message}`)
  }

  if (!response.ok) {
    die(`${preset.id}: npm tarball 下载失败: HTTP ${response.status}`)
  }

  const content = Buffer.from(await response.arrayBuffer())
  if (!matchesSha512Integrity(content, integrity)) {
    die(`${preset.id}: npm tarball SHA-512 完整性校验失败`)
  }

  const tarball = join(temp, `${preset.id}.tgz`)
  writeFileSync(tarball, content)
  return tarball
}

/** 从校验通过的本地 tarball 安装内置 npm 插件，防止 pnpm 再次走网络下载。 */
async function fetchNpmPackage(preset: PresetPlugin, temp: string): Promise<string> {
  if (preset.integrity === undefined) {
    die(`${preset.id}: internal npm 插件必须指定 SHA-512 integrity`)
  }

  let source: ReturnType<typeof parseNpmSource>
  try {
    source = parseNpmSource(preset.spec)
    parseSha512Integrity(preset.integrity)
  }
  catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    die(`${preset.id}: ${message}`)
  }

  const tarball = await downloadVerifiedNpmTarball(preset, source.tarballUrl, preset.integrity, temp)
  const project = join(temp, 'project')
  mkdirSync(project, { recursive: true })
  writeFileSync(join(project, 'package.json'), JSON.stringify({ private: true }))
  run('pnpm', ['add', pathToFileURL(tarball).href, '--ignore-scripts'], project)
  const pkgDir = join(project, 'node_modules', source.packageName)
  if (!existsSync(join(pkgDir, 'package.json'))) {
    die(`${preset.id}: npm 安装后未找到产物 ${pkgDir}`)
  }
  console.log(`[prebuild] ${preset.id}: 来源 npm ${preset.spec}（SHA-512 已验证）`)
  return pkgDir
}

/**
 * 拷贝构建产物：优先 `files` 白名单（只发运行必需：lib/、patch 文件、README），
 * 缺失白名单时拷贝整目录但排除 node_modules/.git 等开发噪声；
 * `package.json` 恒在（它是 `pnpm add file:<dir>` 的包名/入口来源）。
 */
function collectBundle(preset: PresetPlugin, clone: string): void {
  const dest = join(BUNDLE_ROOT, preset.id)
  mkdirSync(dest, { recursive: true })

  const manifest = JSON.parse(readFileSync(join(clone, 'package.json'), 'utf8')) as Record<string, unknown>
  const rawFiles = manifest.files
  const files = Array.isArray(rawFiles)
    ? rawFiles.filter((f): f is string => typeof f === 'string')
    : undefined
  const skip = new Set(['node_modules', '.git', '.gitignore', '.npmrc'])
  const entries = files !== undefined && files.length > 0
    ? files
    : readdirSync(clone).filter(name => !skip.has(name) && !name.endsWith('.tsbuildinfo'))

  for (const name of entries) {
    const src = join(clone, name)
    if (!existsSync(src)) {
      die(`${preset.id}: 白名单产物缺失 ${src}`)
    }
    cpSync(src, join(dest, name), { recursive: true })
  }
  // 拷贝后置，确保即使白名单里没有 package.json 它也一定存在
  cpSync(join(clone, 'package.json'), join(dest, 'package.json'))
}

/** 构建单个 internal 插件：git 来源（克隆 → 装依赖 → 构建）或 npm 来源（拉产物）。 */
async function buildPlugin(preset: PresetPlugin): Promise<void> {
  const dest = join(BUNDLE_ROOT, preset.id)
  rmSync(dest, { recursive: true, force: true })

  const temp = mkdtempSync(join(tmpdir(), `dsh-internal-${preset.id}-`))
  let source: string
  if (preset.spec.startsWith('github:')) {
    const clone = cloneGitHubSource(preset, temp)

    const revision = spawnSync('git', ['-C', clone, 'rev-parse', '--short', 'HEAD'], { encoding: 'utf8' })
    if (revision.status === 0) {
      console.log(`[prebuild] ${preset.id}: 来源修订 ${revision.stdout.trim()}`)
    }

    // 注意：pnpm ≥10 默认拦截依赖的构建脚本（esbuild/原生模块需在插件仓库
    // 的 pnpm-workspace.yaml 配 onlyBuiltDependencies 放行）；纯 JS/TS 插件不受影响。
    run('pnpm', ['install'], clone)
    const manifest = JSON.parse(readFileSync(join(clone, 'package.json'), 'utf8')) as {
      scripts?: Record<string, string>
    }
    if (manifest.scripts?.build !== undefined) {
      run('pnpm', ['run', 'build'], clone)
    }
    source = clone
  }
  else {
    source = await fetchNpmPackage(preset, temp)
  }

  collectBundle(preset, source)
  rmSync(temp, { recursive: true, force: true })
  console.log(`[prebuild] ${preset.id}: 产物已就绪 → ${dest}`)
}

async function main(): Promise<void> {
  if (!existsSync(PRESET_FILE)) {
    die(`未找到预设清单 ${PRESET_FILE}`)
  }
  const presets = JSON.parse(readFileSync(PRESET_FILE, 'utf8')) as PresetPlugin[]
  const internal = presets.filter(preset => preset.internal === true)
  if (internal.length === 0) {
    console.log('[prebuild] 预设清单无 internal 插件，跳过')
    return
  }
  console.log(`[prebuild] 拉取 ${internal.length} 个 internal 插件: ${internal.map(p => p.id).join(', ')}`)
  for (const preset of internal) {
    await buildPlugin(preset)
  }
  console.log(`[prebuild] 完成 → ${BUNDLE_ROOT}`)
}

void main().catch((error: unknown) => {
  const message = error instanceof Error ? error.message : String(error)
  die(message)
})
