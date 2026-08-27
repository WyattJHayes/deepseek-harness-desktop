import { existsSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const loadersPath = new URL('../src/layout/components/lazy-overlays.tsx', import.meta.url)
const navbarSource = readFileSync(new URL('../src/layout/components/navbar.tsx', import.meta.url), 'utf8')
const updaterSource = readFileSync(new URL('../src/layout/components/desktop-updater.tsx', import.meta.url), 'utf8')
const viteSource = readFileSync(new URL('../vite.config.ts', import.meta.url), 'utf8')

describe('desktop code splitting', () => {
  it('loads modal-only views outside the initial shell chunk', () => {
    expect(existsSync(loadersPath)).toBe(true)

    const loadersSource = readFileSync(loadersPath, 'utf8')
    for (const component of ['config-dialog', 'desktop-about-dialog', 'desktop-update-dialog']) {
      expect(loadersSource).toContain(`import('@/components/${component}')`)
    }

    expect(navbarSource).not.toContain('from \'@/components/config-dialog\'')
    expect(navbarSource).not.toContain('from \'@/components/desktop-about-dialog\'')
    expect(navbarSource).not.toContain('from \'@/components/desktop-update-dialog\'')
    expect(updaterSource).not.toContain('from \'@/components/desktop-update-dialog\'')
    expect(viteSource).toContain('manualChunks')
  })
})
