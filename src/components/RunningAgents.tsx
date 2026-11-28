import { useId, useState } from 'react'

type Run = { chatId: string; title: string; work: string }

export function RunningAgents({ runs, onSelect }: { runs: Run[]; onSelect: (chatId: string) => void }) {
  const [open, setOpen] = useState(false)
  const cardId = useId()
  const titleId = useId()
  const label = runs.length ? `Running agents (${runs.length})` : 'Running agents'

  return (
    <>
      <button type="button" className={open ? 'view-tab agents-trigger is-on' : 'view-tab agents-trigger'}
        aria-label={label} aria-haspopup="dialog" aria-expanded={open} aria-controls={cardId}
        popoverTarget={cardId} title={label}>
        <Glyph />
        <span className="view-label">Agents</span>
        {runs.length > 0 && <span className="agent-count" aria-hidden="true">{runs.length}</span>}
      </button>
      <div className="agents-card" id={cardId} popover="auto" role="dialog" aria-labelledby={titleId}
        onToggle={(event) => setOpen(event.newState === 'open')}>
        <header>
          <h2 id={titleId}>Running agents <span>{runs.length}</span></h2>
          <button type="button" className="ghost agents-close" aria-label="Close running agents"
            ref={(button) => { button?.setAttribute('autofocus', '') }} popoverTarget={cardId} popoverTargetAction="hide">
            <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
              <path d="m4 4 8 8M12 4l-8 8" fill="none" stroke="currentColor" strokeWidth="1.4" />
            </svg>
          </button>
        </header>
        {runs.length === 0 ? (
          <p className="agents-empty">No agents running.<span>Start a task to see its progress here.</span></p>
        ) : (
          <ul>
            {runs.map((run) => (
              <li key={run.chatId}>
                <button type="button" title={`${run.title} · ${run.work}`} onClick={(event) => {
                  event.currentTarget.closest<HTMLElement>('[popover]')?.hidePopover()
                  onSelect(run.chatId)
                }}>
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
  )
}

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
