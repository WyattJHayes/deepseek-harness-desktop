import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const builder = readFileSync(new URL('../src-tauri/src/desktop/builder.rs', import.meta.url), 'utf8')

describe('windows WebView2 data directory', () => {
  it('overrides the directory only for debug builds', () => {
    expect(builder).toMatch(/#\[cfg\(all\(windows, debug_assertions\)\)\][\s\S]*?\.data_directory\(/)
    expect(builder).not.toContain('"EBWebView"')
    expect(builder).toContain('"EBWebView-dev"')
  })
})
