import { useEffect, useState } from 'react'
import { emitTo, listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { applyAppearance, loadPreferences } from '../lib/settings'

/// One running turn as the main window reports it.
export type WidgetRun = { chatId: string; title: string; work: string }

/// The widget wears the main window's theme, read from the same stored
/// preferences. It keeps its own size: the app-wide interface scale would clip
/// a 320 px card.
function paint() {
  applyAppearance(loadPreferences(), matchMedia('(prefers-color-scheme: light)').matches)
  const root = document.documentElement.style
  root.removeProperty('zoom')
  root.setProperty('--ui-scale', '1')
}

/// The always-on-top companion window.
///
/// It is a read-only view fed by `nova:widget-state`; clicking a row asks the
/// main window to focus that task. Nothing here starts, stops or approves work.
/// The close button is a request, not a fact: the main window turns the
/// preference off, so the widget stays gone after a restart instead of coming
/// back because a stored toggle still said it was on.
export function AgentWidget() {
  const [runs, setRuns] = useState<WidgetRun[]>([])
  const [paused, setPaused] = useState(false)

  useEffect(() => {
    document.documentElement.dataset.window = 'widget'
    const window = getCurrentWindow()
    const stop = listen<{ runs: WidgetRun[]; paused: boolean }>('nova:widget-state', (event) => {
      setRuns(Array.isArray(event.payload?.runs) ? event.payload.runs.slice(0, 12) : [])
      setPaused(Boolean(event.payload?.paused))
    })
    // A theme change while the widget is up lands on the next time it is looked
    // at, which is the only moment it matters.
    const stopFocus = window.onFocusChanged(({ payload }) => { if (payload) paint() })
    const stopPaint = listen('nova:appearance', () => paint())
    paint()
    return () => {
      void stop.then((off) => off()).catch(() => {})
      void stopFocus.then((off) => off()).catch(() => {})
      void stopPaint.then((off) => off()).catch(() => {})
    }
  }, [])

  /// `close` needs `core:window:allow-close` in this window's capability;
  /// `core:window:default` is read-only, so a missing grant leaves the × inert.
  const dismiss = () => {
    void emitTo('main', 'nova:widget-dismiss').catch(() => {})
    void getCurrentWindow().close().catch(() => {})
  }

  return (
    <main className="agent-widget" aria-label="Running agents">
      <header>
        <span aria-hidden="true">✦</span>
        <strong>Agents</strong>
        <small>{runs.length ? `${runs.length} running` : 'Idle'}</small>
        <button type="button" className="ghost agent-widget-close" aria-label="Close the widget"
          title="Close the widget" onClick={dismiss}>×</button>
      </header>
      {runs.length === 0 ? (
        <p className="agent-widget-empty">
          {paused ? 'No agent is running. Schedules are paused.' : 'No agent is running.'}
        </p>
      ) : (
        <ul>
          {runs.map((run) => (
            <li key={run.chatId}>
              <button type="button" onClick={() => {
                void emitTo('main', 'nova:focus-chat', { chatId: run.chatId }).catch(() => {})
              }}>
                <strong>{run.title}</strong>
                <small>{run.work}</small>
              </button>
            </li>
          ))}
        </ul>
      )}
    </main>
  )
}
