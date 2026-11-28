import { useEffect, useId, useState } from 'react'

type Run = { chatId: string; title: string; work: string }

/// What is running right now, at the end of the viewbar. A row is also how you
/// get to the chat that owns it, which is the whole point of the card.
export function RunningAgents({ runs, onSelect }: { runs: Run[]; onSelect: (chatId: string) => void }) {
  const [open, setOpen] = useState(false)
  const cardId = useId()

  // Escape closes the card, as it does every other overlay in the window.
  useEffect(() => {
    if (!open) return
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpen(false) }
    window.addEventListener('keydown', escape)
    return () => window.removeEventListener('keydown', escape)
  }, [open])

  return (
    <>
      <button
        type="button"
        className={open ? 'view-tab is-on' : 'view-tab'}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? cardId : undefined}
        title={runs.length ? `Running agents (${runs.length})` : 'Running agents'}
        onClick={() => setOpen((current) => !current)}
      >
        <Glyph />
        <span className="view-label">Agents</span>
        {runs.length > 0 && <span className="agent-count">{runs.length}</span>}
      </button>

      {open && (
        <>
          <div className="rail-scrim" onClick={() => setOpen(false)} />
          <div className="agents-card" id={cardId} role="dialog" aria-label="Running agents">
            <header>
              <h2>Running agents</h2>
              <button type="button" className="ghost" aria-label="Close" onClick={() => setOpen(false)}>×</button>
            </header>
            {runs.length === 0 ? (
              <p className="agents-empty">No agents running</p>
            ) : (
              <ul>
                {runs.map((run) => (
                  <li key={run.chatId}>
                    <button type="button" onClick={() => { setOpen(false); onSelect(run.chatId) }}>
                      <span className="agent-live" aria-hidden="true" />
                      <strong>{run.title}</strong>
                      <small>{run.work}</small>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </>
      )}
    </>
  )
}

/// Two figures and a live dot: the run, not a robot. aria-hidden, because the
/// label and the button's title already name the control.
function Glyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="5.5" cy="6" r="2.1" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <circle cx="11" cy="7.5" r="1.7" fill="none" stroke="currentColor" strokeWidth="1.3" />
      <path d="M1.8 13c0-2 1.7-3.3 3.7-3.3S9.2 11 9.2 13M9.8 11.4c1.9-.4 4 .5 4 1.6"
        fill="none" stroke="currentColor" strokeWidth="1.3" />
    </svg>
  )
}
