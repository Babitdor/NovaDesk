import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { closeTerminal, errorText, startTerminal, terminalInput, terminalResize, terminalShells, type TerminalEvent, type TerminalShell } from '../lib/ipc'

const keys: Record<string, string> = {
  Enter: '\r', Backspace: '\x7f', Tab: '\t', Escape: '\x1b',
  ArrowUp: '\x1b[A', ArrowDown: '\x1b[B', ArrowRight: '\x1b[C', ArrowLeft: '\x1b[D',
  Home: '\x1b[H', End: '\x1b[F', Delete: '\x1b[3~', PageUp: '\x1b[5~', PageDown: '\x1b[6~',
}

export function InteractiveTerminal({ active, shell }: { active: boolean; shell: TerminalShell }) {
  const frame = useRef<HTMLDivElement>(null)
  const surface = useRef<HTMLPreElement>(null)
  const input = useRef<HTMLTextAreaElement>(null)
  const process = useRef<string | null>(null)
  const inputQueue = useRef(Promise.resolve())
  const following = useRef(true)
  const [screen, setScreen] = useState<Extract<TerminalEvent, { type: 'screen' }> | null>(null)
  const [status, setStatus] = useState(() => `Starting ${terminalShells[shell]}…`)
  const [alive, setAlive] = useState(false)

  useEffect(() => {
    if (!frame.current || !surface.current) return
    const element = frame.current
    const output = surface.current
    const measure = document.createElement('canvas').getContext('2d')
    const size = () => {
      const style = getComputedStyle(output)
      if (measure) measure.font = `${style.fontSize} ${style.fontFamily}`
      const column = measure?.measureText('0').width || 8
      const row = Number.parseFloat(style.lineHeight) || 18
      return {
        rows: Math.min(100, Math.max(4, Math.floor((element.clientHeight - 16) / row))),
        cols: Math.min(240, Math.max(20, Math.floor((element.clientWidth - 16) / column))),
      }
    }
    const id = crypto.randomUUID()
    let disposed = false
    let started = false
    let exited = false
    let lastSize = size()
    const resize = () => {
      if (disposed || !started || exited || !element.clientWidth || !element.clientHeight) return
      const next = size()
      if (next.rows === lastSize.rows && next.cols === lastSize.cols) return
      lastSize = next
      void terminalResize(id, next.rows, next.cols).catch(() => {})
    }
    const receive = (event: TerminalEvent) => {
      if (disposed) return
      if (event.type === 'screen') {
        setScreen(event)
        setStatus('')
        process.current = id
        setAlive(true)
      } else if (event.type === 'error') {
        setStatus(event.message)
      } else {
        exited = true
        process.current = null
        setAlive(false)
        setStatus((message) => message || 'Shell exited.')
      }
    }
    // React may discard the first mount before the terminal is visible.
    void Promise.resolve().then(async () => {
      if (disposed) return
      await startTerminal(id, shell, lastSize.rows, lastSize.cols, receive)
      if (disposed) {
        void closeTerminal(id).catch(() => {})
      } else {
        started = true
        resize()
      }
    }).catch((error) => { if (!disposed) setStatus(errorText(error)) })

    const observer = new ResizeObserver(resize)
    observer.observe(element)
    void document.fonts.ready.then(resize)
    return () => {
      disposed = true
      if (process.current === id) process.current = null
      observer.disconnect()
      if (started) void closeTerminal(id).catch(() => {})
    }
  }, [shell])

  useLayoutEffect(() => {
    if (active && alive) input.current?.focus({ preventScroll: true })
  }, [active, alive])

  useLayoutEffect(() => {
    const element = frame.current
    if (active && element && following.current) element.scrollTop = element.scrollHeight
  }, [active, screen])

  const write = (value: string) => {
    const id = process.current
    if (!id || !value) return
    following.current = true
    if (frame.current) frame.current.scrollTop = frame.current.scrollHeight
    inputQueue.current = inputQueue.current.then(async () => {
      if (process.current !== id) return
      await terminalInput(id, value)
    }).catch((error) => {
      if (process.current === id) setStatus(errorText(error))
    })
  }

  return (
    <div className="interactive-terminal" ref={frame} hidden={!active}
      onScroll={(event) => {
        const el = event.currentTarget
        following.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24
      }}
      onClick={() => { if (!window.getSelection()?.toString()) input.current?.focus({ preventScroll: true }) }}>
      <pre className="interactive-screen" aria-label="Terminal output" ref={surface}>
        {screen?.lines.map((line, row) => (
          <span className="terminal-line" key={row}>{line || ' '}
            {screen.row === row && alive && <i aria-hidden="true" style={{ left: `${screen.column}ch` }} />}
          </span>
        ))}
      </pre>
      <textarea className="interactive-input" aria-label={`${terminalShells[shell]} input. Use Shift Tab to leave the terminal.`}
        ref={input} disabled={!alive} spellCheck={false} autoCapitalize="off" autoComplete="off"
        onChange={(event) => {
          if ((event.nativeEvent as InputEvent).isComposing) return
          write(event.currentTarget.value)
          event.currentTarget.value = ''
        }}
        onCompositionEnd={(event) => {
          write(event.currentTarget.value)
          event.currentTarget.value = ''
        }}
        onPaste={(event) => { event.preventDefault(); write(event.clipboardData.getData('text')) }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing || (event.shiftKey && event.key === 'Tab')) return
          const letter = event.key.toLowerCase()
          if (event.metaKey || (event.ctrlKey && (letter === 'v' || (letter === 'c' && window.getSelection()?.toString())))) return
          const control = event.ctrlKey && /^[a-z]$/.test(letter) && !event.altKey
            ? String.fromCharCode(letter.charCodeAt(0) - 96) : keys[event.key]
          if (!control) return
          event.preventDefault()
          event.stopPropagation()
          write(control)
        }} />
      {status && <p className="terminal-notice" role="status">{status}</p>}
    </div>
  )
}
