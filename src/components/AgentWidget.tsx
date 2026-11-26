import { useEffect, useState } from 'react'
import { emitTo, listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

/// One running turn as the main window reports it.
export type WidgetRun = { chatId: string; title: string; work: string }

/// The always-on-top companion window.
///
/// It is a read-only view fed by `nova:widget-state`; clicking a row asks the
/// main window to focus that task. Nothing here starts, stops or approves work.
export function AgentWidget() {
  const [runs, setRuns] = useState<WidgetRun[]>([])
  const [paused, setPaused] = useState(false)

  useEffect(() => {
    document.documentElement.dataset.window = 'widget'
    const stop = listen<{ runs: WidgetRun[]; paused: boolean }>('nova:widget-state', (event) => {
      setRuns(Array.isArray(event.payload?.runs) ? event.payload.runs.slice(0, 12) : [])
      setPaused(Boolean(event.payload?.paused))
    })
    return () => { void stop.then((off) => off()).catch(() => {}) }
  }, [])

  return (
    <main className="agent-widget" aria-label="Running agents">
      <header>
        <span aria-hidden="true">✦</span>
        <strong>Agents</strong>
        <small>{runs.length ? `${runs.length} running` : 'Idle'}</small>
        <button type="button" className="ghost agent-widget-close" aria-label="Close the widget"
          onClick={() => { void getCurrentWindow().close().catch(() => {}) }}>×</button>
      </header>
      {runs.length === 0 && <p className="agent-widget-empty">No agent is running.</p>}
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
      {paused && <p className="agent-widget-empty">Schedules are paused.</p>}
    </main>
  )
}
