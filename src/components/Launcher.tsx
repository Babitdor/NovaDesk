import { useEffect, useMemo, useRef, useState } from 'react'
import { emitTo, listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { applyAppearance, loadPreferences } from '../lib/settings'

/// One running turn, as the main window reports it.
type LauncherRun = { chatId: string; title: string; work: string }
type LauncherState = { workspace: string; model: string; runs: LauncherRun[] }

/// The palette's colours come from the same appearance pass the main window
/// runs, read from the same stored preferences. The interface scale is dropped:
/// the window has a fixed size and scaling it would only clip the cards.
function paint() {
  applyAppearance(loadPreferences(), matchMedia('(prefers-color-scheme: light)').matches)
  const root = document.documentElement.style
  root.removeProperty('zoom')
  root.setProperty('--ui-scale', '1')
}

/// The quick launcher window.
///
/// It has no IPC commands of its own — the capability grants only events and
/// window controls — so it cannot read a workspace, run a turn or touch the
/// filesystem. It asks the main window what to show, hands one prompt back and
/// disappears; the main window decides whether the task can start.
export function Launcher() {
  const [text, setText] = useState('')
  const [find, setFind] = useState('')
  const [state, setState] = useState<LauncherState>({ workspace: '', model: '', runs: [] })
  const prompt = useRef<HTMLTextAreaElement>(null)

  /// Hiding is a window permission, not a given: `capabilities/windows.json`
  /// grants this window `core:window:allow-hide`, and without it the promise
  /// rejects, so every attempt to close the palette does nothing.
  const hide = () => void getCurrentWindow().hide().catch(() => {})

  const submit = async () => {
    const task = text.trim()
    if (!task) return
    try {
      await emitTo('main', 'nova:quick-task', { prompt: task })
      setText('')
    } catch {
      // The main window is gone: closing is the honest outcome.
    }
    hide()
  }

  useEffect(() => {
    document.documentElement.dataset.window = 'launcher'
    const window = getCurrentWindow()
    // Escape is bound to the document, not to the panel: the window is created
    // once and reshown, so focus can sit outside the prompt, and a handler on an
    // element only sees keys that element is given.
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      hide()
    }
    // Each show re-focuses the prompt, re-reads the theme and asks for the
    // header's facts: the component mounts once, the window appears many times.
    const onFocus = (focused: boolean) => {
      if (!focused) return
      paint()
      prompt.current?.focus()
      void emitTo('main', 'nova:launcher-ready').catch(() => {})
    }
    const stopFocus = window.onFocusChanged(({ payload }) => onFocus(payload))
    // A theme change while the palette is up: the main window says so.
    const stopPaint = listen('nova:appearance', () => paint())
    const stopState = listen<LauncherState>('nova:launcher-state', (event) => {
      setState({
        workspace: String(event.payload?.workspace ?? ''),
        model: String(event.payload?.model ?? ''),
        runs: Array.isArray(event.payload?.runs) ? event.payload.runs.slice(0, 12) : [],
      })
    })
    document.addEventListener('keydown', onKey)
    onFocus(true)
    return () => {
      document.removeEventListener('keydown', onKey)
      void stopFocus.then((off) => off()).catch(() => {})
      void stopPaint.then((off) => off()).catch(() => {})
      void stopState.then((off) => off()).catch(() => {})
    }
  }, [])

  const runs = useMemo(() => {
    const needle = find.trim().toLowerCase()
    if (!needle) return state.runs
    return state.runs.filter((run) => (run.title + ' ' + run.work).toLowerCase().includes(needle))
  }, [state.runs, find])

  return (
    <main className="launcher" aria-label="Quick launcher">
      <header className="launcher-head">
        <span className="launcher-mark" aria-hidden="true">✦</span>
        <strong>Nova</strong>
        <button type="button" className="launcher-esc" onClick={hide} title="Close (Esc)">
          esc
        </button>
      </header>

      <section className="launcher-card">
        <textarea
          ref={prompt}
          value={text}
          rows={3}
          placeholder="What should we work on?"
          aria-label="Task prompt"
          maxLength={12000}
          spellCheck={false}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.shiftKey) return
            event.preventDefault()
            void submit()
          }}
        />
        <div className="launcher-tools">
          <span className="launcher-chip" title={state.workspace || 'No folder is open'}>
            <span aria-hidden="true">▤</span>
            {state.workspace || 'No folder'}
          </span>
          <span className="launcher-chip" title={state.model || 'Default model'}>
            <span aria-hidden="true">◈</span>
            {state.model || 'Default model'}
          </span>
          <button type="button" className="launcher-go" disabled={!text.trim()}
            onClick={() => void submit()} aria-label="Start task" title="Start task (Enter)">
            <span aria-hidden="true">↑</span>
          </button>
        </div>
      </section>

      <section className="launcher-card launcher-agents">
        <div className="launcher-agents-head">
          <strong>Active agents</strong>
          <label className="launcher-find">
            <span aria-hidden="true">⌕</span>
            <input value={find} onChange={(event) => setFind(event.target.value)}
              placeholder="Find a task" aria-label="Find a task" spellCheck={false} />
          </label>
        </div>
        {runs.length === 0 ? (
          <p className="launcher-empty">
            {state.runs.length ? 'No task matches that search.' : 'No agents working right now.'}
          </p>
        ) : (
          <ul className="launcher-runs">
            {runs.map((run) => (
              <li key={run.chatId}>
                <button type="button" onClick={() => {
                  void emitTo('main', 'nova:focus-chat', { chatId: run.chatId }).catch(() => {})
                  hide()
                }}>
                  <strong>{run.title}</strong>
                  <small>{run.work}</small>
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>

      <footer className="launcher-foot">
        <span className="launcher-hint"><kbd>↵</kbd> start task</span>
        <span className="launcher-hint"><kbd>⇧↵</kbd> new line</span>
        <span className="launcher-live" data-on={runs.length > 0}>
          <i aria-hidden="true" /> Live status
        </span>
      </footer>
    </main>
  )
}
