import { useRef, useState } from 'react'

import type { AgentEvent, OutputStream } from '../lib/events'
import { errorText, killProcess, runCommand } from '../lib/ipc'

type Line = { id: number; stream: OutputStream; chunk: string }

/// Cap the transcript: a dev server can run for hours.
const MAX_LINES = 800

export function ConsolePanel() {
  const [command, setCommand] = useState('')
  const [lines, setLines] = useState<Line[]>([])
  const [running, setRunning] = useState<string | null>(null)
  const [status, setStatus] = useState<string | null>(null)
  const nextLineId = useRef(0)

  const start = async () => {
    const [program, ...args] = command.trim().split(/\s+/)
    if (!program || running) return

    const commandId = crypto.randomUUID()
    setLines([])
    setStatus(null)
    setRunning(commandId)

    const onEvent = (event: AgentEvent) => {
      if (event.type === 'processOutput') {
        const line = { id: nextLineId.current++, stream: event.stream, chunk: event.chunk }
        setLines((prev) => [...prev, line].slice(-MAX_LINES))
      } else if (event.type === 'toolResult' && event.callId === commandId) {
        setStatus(event.output)
        setRunning(null)
      }
    }

    try {
      await runCommand(commandId, program, args, onEvent)
    } catch (err) {
      setStatus(errorText(err))
      setRunning(null)
    }
  }

  return (
    <section className="console">
      <header className="console-head">
        <span>Console</span>
        {running ? (
          <button type="button" className="ghost" onClick={() => void killProcess(running)}>
            Kill
          </button>
        ) : (
          <span className="console-status">{status}</span>
        )}
      </header>

      <div className="console-input">
        <input
          value={command}
          placeholder="cmd /C npm run build"
          onChange={(event) => setCommand(event.target.value)}
          onKeyDown={(event) => event.key === 'Enter' && void start()}
        />
        <button type="button" className="send" onClick={() => void start()} disabled={!!running}>
          Run
        </button>
      </div>

      <pre className="console-output">
        {lines.length === 0 && <span className="console-idle">no output yet</span>}
        {lines.map((line) => (
          <span key={line.id} className={line.stream === 'stderr' ? 'stderr' : undefined}>
            {line.chunk}
          </span>
        ))}
      </pre>
    </section>
  )
}
