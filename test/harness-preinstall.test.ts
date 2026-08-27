import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(async () => vi.fn()),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }))
vi.mock('@hairy/react-lib', () => ({
  emitter: { emit: vi.fn() },
}))
vi.mock('@/config/client', () => ({
  queryClient: { invalidateQueries: vi.fn() },
}))
vi.mock('@/store/modules/harness-updater', () => ({
  harnessUpdater: { checkForUpdate: vi.fn() },
}))

const { harness } = await import('../src/store/modules/harness/store')

function rejectHarnessLaunch(command: string): Promise<unknown> {
  if (command === 'launch_harness')
    return Promise.reject(new Error('launch failed after preinstall'))
  if (command === 'read_service_logs')
    return Promise.resolve('')
  return Promise.resolve(undefined)
}

async function flushMicrotasks(): Promise<void> {
  for (let index = 0; index < 10; index++)
    await Promise.resolve()
}

describe('preinstall startup transition', () => {
  beforeEach(() => {
    mocks.invoke.mockReset()
    mocks.invoke.mockImplementation(rejectHarnessLaunch)
    mocks.listen.mockClear()
    harness.status = 'preinstall'
    harness.errorMsg = ''
    harness.errorLogs = []
    harness.serviceHealthy = false
    harness.serviceRunning = false
    harness.preinstall.error = ''
    harness.preinstall.installing = false
    harness.preinstall.cancelling = false
    harness.preinstall.logs = []
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('shows the global retryable error page when launch fails after skipping', async () => {
    await harness.skipPreinstall()

    expect(harness.status).toBe('error')
    expect(harness.errorMsg).toContain('launch failed after preinstall')
    expect(harness.preinstall.error).toBe('')
  })

  it('shows the global retryable error page when launch fails after installation', async () => {
    await harness.confirmPreinstall(['dshmarket'])

    expect(harness.status).toBe('error')
    expect(harness.errorMsg).toContain('launch failed after preinstall')
    expect(harness.preinstall.error).toBe('')
    expect(harness.preinstall.installing).toBe(false)
  })

  it('does not let an older launch overwrite a newer startup failure', async () => {
    const firstLaunch = Promise.withResolvers<void>()
    let launchCalls = 0
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'launch_harness') {
        launchCalls++
        if (launchCalls === 1)
          return firstLaunch.promise
        return Promise.reject(new Error('newer launch failed'))
      }
      if (command === 'get_runtime_info')
        return Promise.resolve({ service_url: 'http://127.0.0.1:3081' })
      if (command === 'read_service_logs')
        return Promise.resolve('')
      return Promise.resolve(undefined)
    })

    const staleTransition = harness.skipPreinstall()
    await flushMicrotasks()
    expect(launchCalls).toBe(1)

    await harness.skipPreinstall()
    expect(harness.status).toBe('error')
    expect(harness.serviceRunning).toBe(false)

    firstLaunch.resolve()
    await staleTransition

    expect(harness.status).toBe('error')
    expect(harness.serviceRunning).toBe(false)
  })

  it('recovers in the background when a slow service becomes ready after timeout', async () => {
    vi.useFakeTimers()
    const delayedHealth = Promise.withResolvers<string>()
    let probes = 0
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'get_runtime_info')
        return Promise.resolve({ service_url: 'http://127.0.0.1:3081' })
      if (command === 'proxy_health_check') {
        probes++
        return probes <= 8 ? Promise.resolve('starting') : delayedHealth.promise
      }
      if (command === 'read_service_logs')
        return Promise.resolve('')
      return Promise.resolve(undefined)
    })

    const transition = harness.skipPreinstall()
    await vi.advanceTimersByTimeAsync(14_000)
    await flushMicrotasks()

    expect(probes).toBe(9)
    expect(harness.status).toBe('error')
    expect(harness.serviceRunning).toBe(true)

    delayedHealth.resolve('healthy')
    await transition
    await flushMicrotasks()

    expect(harness.status).toBe('ready')
    expect(harness.serviceHealthy).toBe(true)
    expect(harness.serviceRunning).toBe(true)
  })
})
