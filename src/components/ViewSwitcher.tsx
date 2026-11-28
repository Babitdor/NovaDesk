import { VIEWS, type View } from '../lib/panes'
import type { Dock } from './TerminalDock'

type Props = {
  view: View
  dock: Dock
  onSelect: (view: View) => void
  onToggleDock: () => void
}

/// The tooltip, which is where the shortcut and the headmate's name live. "Chat"
/// is the label on screen because that is what the column is; Schrodinger is what
/// the wordmark calls it.
const TITLES: Record<View, string> = {
  Chat: 'Chat — Schrodinger',
  Files: 'Files',
  GitHub: 'GitHub',
  Board: 'Board',
  Preview: 'Preview',
}

/// View navigation lives in the floating rail; Terminal toggles its own dock.
export function ViewSwitcher({ view, dock, onSelect, onToggleDock }: Props) {
  return (
    <>
      <div className="view-tabs" role="tablist" aria-label="Views" aria-orientation="vertical" onKeyDown={(event) => {
        const index = VIEWS.indexOf(view)
        const next = event.key === 'ArrowDown' ? (index + 1) % VIEWS.length
          : event.key === 'ArrowUp' ? (index + VIEWS.length - 1) % VIEWS.length
          : event.key === 'Home' ? 0 : event.key === 'End' ? VIEWS.length - 1 : -1
        if (next < 0) return
        event.preventDefault()
        onSelect(VIEWS[next])
        event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus()
      }}>
        {VIEWS.map((name, index) => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-label={name}
            tabIndex={name === view ? 0 : -1}
            aria-selected={name === view}
            className={name === view ? 'view-tab is-on' : 'view-tab'}
            title={`${TITLES[name]} (Ctrl + ${index + 1})`}
            onClick={() => onSelect(name)}
          >
            <Glyph view={name} />
            <span className="view-label">{name}</span>
          </button>
        ))}
      </div>

      <span className="view-rule" aria-hidden="true" />

      <button
        type="button"
        className={dock === 'closed' ? 'view-tab' : 'view-tab is-on'}
        aria-label="Terminal"
        aria-pressed={dock !== 'closed'}
        title="Terminal (Ctrl + `)"
        onClick={onToggleDock}
      >
        <span className="view-glyph" aria-hidden="true">{'>_'}</span>
        <span className="view-label">Terminal</span>
      </button>
    </>
  )
}

/// One drawn glyph per view, at the weight of the labels beside them. They are
/// aria-hidden: the label is the name, so an icon never has to be read out twice.
function Glyph({ view }: { view: View }) {
  const shape = () => {
    switch (view) {
      case 'Chat':
        return <path d="M2.5 3h11v7H7L4 12.5V10H2.5z" fill="none" stroke="currentColor" strokeWidth="1.4" />
      case 'Files':
        return <path d="M2 12V4h4l1.5 1.5H14V12z" fill="none" stroke="currentColor" strokeWidth="1.4" />
      case 'GitHub':
        return (
          <>
            <circle cx="4.5" cy="4" r="1.7" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <circle cx="4.5" cy="12" r="1.7" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <circle cx="11.5" cy="6.5" r="1.7" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <path d="M4.5 5.7v4.6M11.5 8.2c0 2-2 2.4-4 2.6" fill="none" stroke="currentColor" strokeWidth="1.3" />
          </>
        )
      case 'Board':
        return (
          <>
            <rect x="2" y="3" width="3.5" height="10" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <rect x="6.25" y="3" width="3.5" height="6.5" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <rect x="10.5" y="3" width="3.5" height="8" fill="none" stroke="currentColor" strokeWidth="1.3" />
          </>
        )
      case 'Preview':
        return (
          <>
            <circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
            <path d="M2.5 8h11M8 2.5c2.2 2.2 2.2 8.8 0 11M8 2.5c-2.2 2.2-2.2 8.8 0 11"
              fill="none" stroke="currentColor" strokeWidth="1.2" />
          </>
        )
    }
  }

  return (
    <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
      {shape()}
    </svg>
  )
}
