import type { ConfigDialogProps } from '@/components/config-dialog'
import type { DesktopAboutDialogProps } from '@/components/desktop-about-dialog'
import type { DesktopUpdateDialogProps } from '@/components/desktop-update-dialog'
import { lazy, Suspense } from 'react'

const ConfigDialogView = lazy(() => import('@/components/config-dialog').then(({ ConfigDialog }) => ({ default: ConfigDialog })))
const DesktopAboutDialogView = lazy(() => import('@/components/desktop-about-dialog').then(({ DesktopAboutDialog }) => ({ default: DesktopAboutDialog })))
const DesktopUpdateDialogView = lazy(() => import('@/components/desktop-update-dialog').then(({ DesktopUpdateDialog }) => ({ default: DesktopUpdateDialog })))

export function ConfigDialogOverlay(props: ConfigDialogProps) {
  return (
    <Suspense fallback={null}>
      <ConfigDialogView {...props} />
    </Suspense>
  )
}

export function DesktopAboutDialogOverlay(props: DesktopAboutDialogProps) {
  return (
    <Suspense fallback={null}>
      <DesktopAboutDialogView {...props} />
    </Suspense>
  )
}

export function DesktopUpdateDialogOverlay(props: DesktopUpdateDialogProps) {
  return (
    <Suspense fallback={null}>
      <DesktopUpdateDialogView {...props} />
    </Suspense>
  )
}
