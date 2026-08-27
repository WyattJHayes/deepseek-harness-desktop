import { Buffer } from 'node:buffer'
import { createHash, timingSafeEqual } from 'node:crypto'

export interface GitHubSource {
  remoteUrl: string
  revision: string
}

export interface NpmSource {
  packageName: string
  version: string
  tarballUrl: string
}

const GITHUB_SOURCE_RE = /^github:(\w[\w.-]*\/\w[\w.-]*)(?:#([^#]+))?$/
const FULL_GIT_SHA_RE = /^[0-9a-f]{40}$/i
const NPM_PACKAGE_NAME_RE = /^(?:[a-z0-9][a-z0-9._-]*|@[a-z0-9][a-z0-9._-]*\/[a-z0-9][a-z0-9._-]*)$/
const NPM_EXACT_VERSION_RE = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Z-]+(?:\.[0-9A-Z-]+)*)?(?:\+[0-9A-Z-]+(?:\.[0-9A-Z-]+)*)?$/i
const SHA512_SRI_RE = /^sha512-([A-Za-z0-9+/]+={0,2})$/

/** 解析内置插件的 GitHub 来源；若指定 revision，必须是不可变的完整 commit SHA。 */
export function parseGitHubSource(spec: string): GitHubSource {
  const match = GITHUB_SOURCE_RE.exec(spec)
  if (match === null) {
    throw new Error(`GitHub source must use github:owner/repository: ${spec}`)
  }

  const repository = match[1]
  const revision = match[2]
  if (revision === undefined || !FULL_GIT_SHA_RE.test(revision)) {
    throw new Error(`GitHub source revision must be a full 40-character commit SHA: ${spec}`)
  }

  return {
    remoteUrl: `https://github.com/${repository}.git`,
    revision: revision.toLowerCase(),
  }
}

/** 解析内置 npm 插件来源，只允许不可变的精确语义化版本。 */
export function parseNpmSource(spec: string): NpmSource {
  const separator = spec.startsWith('@') ? spec.lastIndexOf('@') : spec.indexOf('@')
  if (separator <= 0 || separator === spec.length - 1) {
    throw new Error(`npm source must use an exact package version: ${spec}`)
  }

  const packageName = spec.slice(0, separator)
  const version = spec.slice(separator + 1)
  if (!NPM_PACKAGE_NAME_RE.test(packageName) || !NPM_EXACT_VERSION_RE.test(version)) {
    throw new Error(`npm source must use an exact package version: ${spec}`)
  }

  const packageBasename = packageName.slice(packageName.lastIndexOf('/') + 1)
  return {
    packageName,
    version,
    tarballUrl: `https://registry.npmjs.org/${encodeURIComponent(packageName)}/-/${encodeURIComponent(`${packageBasename}-${version}.tgz`)}`,
  }
}

/** 解析并严格校验 npm 的 canonical SHA-512 SRI 完整性值。 */
export function parseSha512Integrity(integrity: string): Buffer {
  const match = SHA512_SRI_RE.exec(integrity)
  if (match === null) {
    throw new Error(`npm integrity must use canonical sha512 SRI: ${integrity}`)
  }

  const digest = Buffer.from(match[1], 'base64')
  if (digest.length !== 64 || digest.toString('base64') !== match[1]) {
    throw new Error(`npm integrity must use canonical sha512 SRI: ${integrity}`)
  }
  return digest
}

/** 比较下载字节与固定 SHA-512 SRI，使用常量时间比较避免泄露前缀匹配信息。 */
export function matchesSha512Integrity(content: Uint8Array, integrity: string): boolean {
  const expected = parseSha512Integrity(integrity)
  const actual = createHash('sha512').update(content).digest()
  return timingSafeEqual(actual, expected)
}
