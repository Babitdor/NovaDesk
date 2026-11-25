import { flushSync } from 'react-dom'

/// The slice of the View Transition API the shell uses, so this does not
/// depend on a lib.dom build that already declares it.
type TransitionHost = Document & {
  startViewTransition?: (update: () => void) => unknown
}

/// Hands one rendered state to the next through the platform's snapshot
/// crossfade, which is how a task switch reads as a page turn instead of a
/// repaint. Both webviews the app ships in (WebView2, WKWebView) have it, but
/// it stays an enhancement: without the API, or with reduced motion asked for,
/// this is a plain state update.
export function withViewTransition(update: () => void) {
  const host = document as TransitionHost
  if (typeof host.startViewTransition !== 'function' || matchMedia('(prefers-reduced-motion: reduce)').matches) {
    update()
    return
  }
  // flushSync, not a bare call: React 19 batches the update, and a commitment
  // after the browser has taken its snapshots would crossfade two frames that
  // are already the same one.
  host.startViewTransition(() => flushSync(update))
}
