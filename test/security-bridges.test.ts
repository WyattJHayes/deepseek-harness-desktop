import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

const builderSource = readFileSync(
  new URL('../src-tauri/src/desktop/builder.rs', import.meta.url),
  'utf8',
)
const notificationSource = readFileSync(
  new URL('../src-tauri/src/desktop/notification.rs', import.meta.url),
  'utf8',
)
const iframeShimSource = readFileSync(
  new URL('../src/hooks/use-iframe-shim.ts', import.meta.url),
  'utf8',
)

describe('secure desktop bridges', () => {
  it('registers Windows bridge scripts before page scripts run', () => {
    const lateWindowsRegistration = [
      '#[cfg(not(windows))]',
      '    let webview_builder = webview_builder',
      '        .initialization_script_for_all_frames(crate::desktop::compat::ABORT_SIGNAL_ANY_SHIM_JS)',
      '        .initialization_script_for_all_frames(crate::desktop::notification::NOTIFICATION_SHIM_JS)',
    ].join('\n')

    expect(builderSource).not.toContain(lateWindowsRegistration)
    expect(notificationSource).not.toContain('FrameContentLoadingEventHandler')
    expect(notificationSource).not.toContain('ExecuteScriptCompletedHandler')
  })

  it('uses camelCase for the Tauri issuedAt command argument', () => {
    expect(iframeShimSource).toContain('      issuedAt,\n      proof,')
    expect(iframeShimSource).not.toContain('issued_at: issuedAt')
  })
})
