import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const workflow = readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8')

describe('ci workflow', () => {
  it('runs frontend checks on Windows', () => {
    const start = workflow.indexOf('\n  windows-frontend:\n')
    const end = workflow.indexOf('\n  rust-test:', start)
    expect(start).toBeGreaterThanOrEqual(0)
    expect(end).toBeGreaterThan(start)

    const windowsJob = workflow.slice(start, end)
    expect(windowsJob).toContain('runs-on: windows-latest')
    expect(windowsJob).toContain('pnpm install --frozen-lockfile')
    expect(windowsJob).toContain('pnpm run typecheck')
    expect(windowsJob).toContain('pnpm run test -- --run')
  })
})
