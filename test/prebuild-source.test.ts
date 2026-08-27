import { describe, expect, it } from 'vitest'
import { parseGitHubSource } from '../scripts/preset-source'

describe('parseGitHubSource', () => {
  it('preserves an immutable GitHub revision for the prebuild checkout', () => {
    expect(
      parseGitHubSource(
        'github:dsh-tauri-desk/dsh-tauri-panel#2743acba2265599209478db211418cfd91f74daa',
      ),
    ).toEqual({
      remoteUrl: 'https://github.com/dsh-tauri-desk/dsh-tauri-panel.git',
      revision: '2743acba2265599209478db211418cfd91f74daa',
    })
  })

  it('rejects a mutable branch name as a revision', () => {
    expect(() => parseGitHubSource('github:dsh-tauri-desk/dsh-tauri-panel#main')).toThrow(
      'full 40-character commit SHA',
    )
  })

  it('rejects an unpinned GitHub source', () => {
    expect(() => parseGitHubSource('github:dsh-tauri-desk/dsh-tauri-panel')).toThrow(
      'full 40-character commit SHA',
    )
  })

  it('rejects malformed repository paths', () => {
    expect(() => parseGitHubSource('github:dsh-tauri-desk/../dsh-tauri-panel')).toThrow(
      'github:owner/repository',
    )
  })
})
