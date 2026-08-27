import { Buffer } from 'node:buffer'
import { describe, expect, it } from 'vitest'
import * as presetSource from '../scripts/preset-source'

interface NpmSource {
  packageName: string
  version: string
  tarballUrl: string
}

const { matchesSha512Integrity, parseNpmSource, parseSha512Integrity } = presetSource as unknown as {
  matchesSha512Integrity: (content: Uint8Array, integrity: string) => boolean
  parseNpmSource: (spec: string) => NpmSource
  parseSha512Integrity: (integrity: string) => Buffer
}

describe('parseNpmSource', () => {
  it('accepts an unscoped package with an exact version', () => {
    expect(parseNpmSource('dsh-tauri@0.2.1')).toEqual({
      packageName: 'dsh-tauri',
      version: '0.2.1',
      tarballUrl: 'https://registry.npmjs.org/dsh-tauri/-/dsh-tauri-0.2.1.tgz',
    })
  })

  it('accepts a scoped package with an exact version', () => {
    expect(parseNpmSource('@dsh/tauri-ui@1.2.3')).toEqual({
      packageName: '@dsh/tauri-ui',
      version: '1.2.3',
      tarballUrl: 'https://registry.npmjs.org/%40dsh%2Ftauri-ui/-/tauri-ui-1.2.3.tgz',
    })
  })

  it('rejects mutable or incomplete npm specs', () => {
    for (const spec of [
      'dsh-tauri',
      'dsh-tauri@latest',
      'dsh-tauri@^0.2.1',
      'dsh-tauri@0.2',
      'github:dsh-tauri-desk/dsh-tauri',
      '@dsh/tauri-ui',
    ]) {
      expect(() => parseNpmSource(spec)).toThrow('exact package version')
    }
  })
})

describe('sha-512 integrity', () => {
  const integrity = 'sha512-G6lIAM9CcLVLPrYrvUxnzkJ3eCByiDM0y38WoVsT1FUyoBsrnWLP97UCnYrkQ8EjVDsu9Ln8FM5LKWbSx3R6DA=='

  it('accepts the canonical SHA-512 SRI digest used by npm', () => {
    expect(parseSha512Integrity(integrity)).toHaveLength(64)
  })

  it('rejects a non-SHA-512 or malformed integrity value', () => {
    for (const invalid of [
      'sha1-deadbeef',
      'sha512-not_base64',
      'sha512-G6lIAM9CcLVLPrYrvUxnzkJ3eCByiDM0y38WoVsT1FUyoBsrnWLP97UCnYrkQ8EjVDsu9Ln8FM5LKWbSx3R6DA=',
    ]) {
      expect(() => parseSha512Integrity(invalid)).toThrow('canonical sha512 SRI')
    }
  })

  it('matches only the tarball bytes protected by the pinned digest', () => {
    expect(matchesSha512Integrity(Buffer.from('trusted tarball bytes'), integrity)).toBe(true)
    expect(matchesSha512Integrity(Buffer.from('untrusted tarball bytes'), integrity)).toBe(false)
  })
})
