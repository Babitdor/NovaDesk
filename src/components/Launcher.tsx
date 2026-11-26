import { useEffect, useRef, useState } from 'react'
import { emitTo } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

/// The quick launcher window.
///
/// It has no IPC commands of its own — the capability grants only events and
/// window controls — so it cannot read a workspace, run a turn or touch the
/// filesystem. It hands one prompt to the main window and disappears; the main
/// window decides whether the task can start.
export function Launcher() {
  const [text, setText] = useState('')
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => {
    document.documentElement.dataset.window = 'launcher'
    input.current?.focus()
  }, [])

  const hide = () => void getCurrentWindow().hide().catch(() => {})

  const submit = async () => {
    const prompt = text.trim()
    if (!prompt) return
    try {
      await emitTo('main', 'nova:quick-task', { prompt })
      setText('')
      hide()
    } catch {
      // The main window is gone: closing is the honest outcome.
      hide()
    }
  }

  return (
    <main className="launcher" aria-label="Quick launcher"
      onKeyDown={(event) => { if (event.key === 'Escape') hide() }}>
      <span className="launcher-mark" aria-hidden="true">✦</span>
      <input ref={input} value={text} aria-label="Task prompt" placeholder="Ask Nova…"
        maxLength={12000} spellCheck={false}
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => { if (event.key === 'Enter' && !event.shiftKey) void submit() }} />
      <button type="button" className="launcher-go" disabled={!text.trim()} onClick={() => void submit()}>
        Start
      </button>
    </main>
  )
}
