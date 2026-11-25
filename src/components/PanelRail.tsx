import type { Pane } from '../lib/panes'
import type { Dock } from './TerminalDock'

type Props = {
  tab: Pane
  panelOpen: boolean
  dock: Dock
  onSelect: (pane: Pane) => void
  onTogglePanel: () => void
  onToggleDock: () => void
}

/// The strip between the transcript and the panel: one control per pane, so the
/// panel can be closed and the transcript given the whole window.
export function PanelRail({ tab, panelOpen, dock, onSelect, onTogglePanel, onToggleDock }: Props) {
  const isActive = (pane: Pane) => panelOpen && tab === pane
  const button = (on: boolean) => (on ? 'rail-btn is-on' : 'rail-btn')

  return (
    <nav className="panel-rail" aria-label="Panels">
      <button
        type="button"
        className={button(panelOpen)}
        aria-pressed={panelOpen}
        aria-label="Toggle the side panel"
        onClick={onTogglePanel}
      >
        <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true">
          <rect x="2" y="3" width="12" height="10" fill="none" stroke="currentColor" />
          <path d="M6.5 3v10" fill="none" stroke="currentColor" />
        </svg>
      </button>

      <button
        type="button"
        className={button(isActive('Changes'))}
        aria-pressed={isActive('Changes')}
        aria-label="Changes"
        onClick={() => onSelect('Changes')}
      >
        <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true">
          <path
            d="M3 5h3M4.5 3.5v3M9 5h4M9 11h4M3 11h3"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
          />
        </svg>
      </button>

      <button
        type="button"
        className={button(isActive('Files'))}
        aria-pressed={isActive('Files')}
        aria-label="Files"
        onClick={() => onSelect('Files')}
      >
        <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true">
          <path
            d="M2 12V4h4l1.5 1.5H14V12z"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
          />
        </svg>
      </button>

      <button
        type="button"
        className={button(isActive('Browser'))}
        aria-pressed={isActive('Browser')}
        aria-label="Browser"
        onClick={() => onSelect('Browser')}
      >
        <svg width="15" height="15" viewBox="0 0 16 16" aria-hidden="true">
          <circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
          <path
            d="M2.5 8h11M8 2.5c2.2 2.2 2.2 8.8 0 11M8 2.5c-2.2 2.2-2.2 8.8 0 11"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.2"
          />
        </svg>
      </button>

      <span className="rail-rule" />

      <button
        type="button"
        className={button(dock !== 'closed')}
        aria-pressed={dock !== 'closed'}
        aria-label="Terminal"
        title="Toggle terminal (Ctrl + `)"
        onClick={onToggleDock}
      >
        <span className="rail-glyph">&gt;_</span>
      </button>
    </nav>
  )
}
