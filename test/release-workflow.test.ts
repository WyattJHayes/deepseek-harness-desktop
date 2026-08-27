import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { isPrereleaseTag, rewriteReleaseNotes } from '../scripts/release-workflow.mjs'

const workflow = readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8')

describe('release workflow', () => {
  it('classifies stable and prerelease tags', () => {
    expect(isPrereleaseTag('v0.8.4-beta.1')).toBe(true)
    expect(isPrereleaseTag('v0.8.4-rc.2')).toBe(true)
    expect(isPrereleaseTag('v0.8.4')).toBe(false)
  })

  it('propagates prerelease metadata to GitHub CLI and tauri-action', () => {
    expect(workflow).toMatch(/prerelease: \$\{\{ steps\.meta\.outputs\.prerelease \}\}/)
    expect(workflow).toContain('node scripts/release-workflow.mjs prerelease "$tag"')
    expect(workflow).toContain('echo "prerelease=$prerelease" >> "$GITHUB_OUTPUT"')
    expect(workflow).toMatch(/RELEASE_PRERELEASE: \$\{\{ steps\.meta\.outputs\.prerelease \}\}/)
    expect(workflow).toContain('--prerelease')
    expect(workflow).toMatch(/prerelease: \$\{\{ needs\.prepare\.outputs\.prerelease == 'true' \}\}/)
  })

  it('never moves an existing release tag to another commit', () => {
    expect(workflow).not.toContain('github.rest.git.updateRef')
    expect(workflow).not.toContain('force: true')
    expect(workflow).toContain('Existing tag')
    expect(workflow).toContain('does not match release commit')
  })

  it('updates a single stable artifacts block without replacement characters', () => {
    expect(workflow).toContain('node scripts/release-workflow.mjs notes release_notes.md')
    const replacement = String.fromCharCode(0xFFFD).repeat(3)
    const legacy = `## Changes\n\n### ${replacement} Artifacts\n- **macOS (Apple Silicon)**: old\n- **Windows**: old`
    const once = rewriteReleaseNotes(legacy)
    const twice = rewriteReleaseNotes(once)

    expect(twice).toBe(once)
    expect(once).not.toContain(replacement)
    expect(once.match(/<!-- dsh-desktop-artifacts:start -->/g)).toHaveLength(1)
    expect(once.match(/<!-- dsh-desktop-artifacts:end -->/g)).toHaveLength(1)
    expect(once).toContain('### Artifacts')
  })
})
