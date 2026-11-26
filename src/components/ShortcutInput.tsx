import { useEffect, useRef, useState } from 'react'

/// Records one global chord.
///
/// The value is an accelerator string, not a key code, because that is what
/// `tauri-plugin-global-shortcut` parses. A chord with no modifier is refused:
/// registering a bare letter would swallow that key in every application.

const MODIFIER_KEYS = new Set(['Control', 'Shift', 'Alt', 'Meta', 'OS'])

const NAMED: Record<string, string> = {
  Space: 'Space', Enter: 'Enter', Tab: 'Tab', Backspace: 'Backspace', Delete: 'Delete',
  ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
  Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
  Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\',
  Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Backquote: '`',
}

/// `event.code` is the physical key, so a saved chord keeps working after the
/// user switches keyboard layout mid-session.
function acceleratorFrom(event: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.has(event.key)) return null
  const parts: string[] = []
  if (event.ctrlKey || event.metaKey) parts.push('CommandOrControl')
  if (event.altKey) parts.push('Alt')
  if (event.shiftKey) parts.push('Shift')
  const key = keyName(event.code)
  // Modifiers alone are an unfinished chord, and a bare key would capture typing.
  if (!parts.length || !key) return null
  parts.push(key)
  return parts.join('+')
}

function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3)
  if (/^Digit[0-9]$/.test(code)) return code.slice(5)
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code
  return NAMED[code] ?? null
}

export function ShortcutInput({ value, onChange, disabled }: {
  value: string; onChange: (accelerator: string) => void; disabled?: boolean
}) {
  const [recording, setRecording] = useState(false)
  const [hint, setHint] = useState('')
  const button = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!recording) return
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault()
      event.stopPropagation()
      if (event.key === 'Escape') {
        setRecording(false)
        setHint('Recording cancelled.')
        return
      }
      const accelerator = acceleratorFrom(event)
      if (!accelerator) {
        setHint('Hold Ctrl, Alt or Shift and press a key.')
        return
      }
      onChange(accelerator)
      setRecording(false)
      setHint('')
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [recording, onChange])

  return (
    <span className="shortcut-input">
      <button ref={button} type="button" disabled={disabled}
        aria-label="Record shortcut"
        aria-pressed={recording}
        onClick={() => { setRecording((open) => !open); setHint('') }}>
        {recording ? 'Press keys…' : value || 'Not set'}
      </button>
      {hint && <small role="status">{hint}</small>}
    </span>
  )
}
