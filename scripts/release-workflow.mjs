import { readFileSync, writeFileSync } from 'node:fs'
import process from 'node:process'
import { pathToFileURL } from 'node:url'

const START_MARKER = '<!-- dsh-desktop-artifacts:start -->'
const END_MARKER = '<!-- dsh-desktop-artifacts:end -->'
const ARTIFACTS_BLOCK = `${START_MARKER}
### Artifacts
- **macOS (Apple Silicon)**: 下载 \`aarch64.dmg\`
- **macOS (Intel)**: 下载 \`x64.dmg\`
- **Windows**: 下载 \`.exe\`
- **Linux (x64)**: 下载 \`.AppImage\` 或 \`.deb\`（基于 Ubuntu 22.04 构建，兼容 22.04 及更新版本）
${END_MARKER}`

export function isPrereleaseTag(tag) {
  const version = tag.replace(/^v/, '').split('+', 1)[0]
  return version.includes('-')
}

export function rewriteReleaseNotes(body) {
  const markerBlock = /\n*<!-- dsh-desktop-artifacts:start -->[\s\S]*?<!-- dsh-desktop-artifacts:end -->\n*/g
  const legacyBlock = /\n*### [^\n]*Artifacts\s*\n- \*\*macOS \(Apple Silicon\)\*\*:[\s\S]*$/g
  const cleaned = body.replace(markerBlock, '\n').replace(legacyBlock, '').trimEnd()
  return `${cleaned ? `${cleaned}\n\n` : ''}${ARTIFACTS_BLOCK}\n`
}

function main() {
  const [command, value] = process.argv.slice(2)
  if (command === 'prerelease' && value !== undefined) {
    process.stdout.write(`${isPrereleaseTag(value)}\n`)
    return
  }
  if (command === 'notes' && value !== undefined) {
    const body = readFileSync(value, 'utf8')
    writeFileSync(value, rewriteReleaseNotes(body))
    return
  }
  throw new Error('Usage: release-workflow.mjs <prerelease TAG | notes FILE>')
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href)
  main()
