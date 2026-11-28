import { useId, useState } from 'react'
import { APPROVAL_MODES, type ApprovalMode } from '../lib/approval'

export function ApprovalIcon({ mode }: { mode: ApprovalMode }) {
  return <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
    {mode === 'standard' ? <path d="m8 1.8 5 2v4c0 3-5 6.2-5 6.2S3 10.8 3 7.8v-4z" />
      : mode === 'acceptEdits' ? <><path d="m3 10 7.7-7.7 3 3L6 13H3zM9 4l3 3" /></>
      : mode === 'plan' ? <><rect x="3.5" y="3" width="9" height="11" rx="1" /><path d="M6 3V1.5h4V3M6 7h4M6 10h4" /></>
      : <><rect x="3.5" y="7" width="9" height="6.5" rx="1" /><path d="M5.7 7V4.8a2.3 2.3 0 0 1 4.6 0" /></>}
  </svg>
}

export function ApprovalPicker({ mode, onChange, planReason }: {
  mode: ApprovalMode; onChange: (mode: ApprovalMode) => void; planReason?: string
}) {
  const id = useId()
  const [open, setOpen] = useState(false)
  const selected = APPROVAL_MODES.find(option => option.id === mode)!
  return <>
    <button type="button" className="ghost approval-trigger" data-mode={mode}
      aria-label={`Approval mode: ${selected.label}`} title={`${selected.label}: ${selected.detail}`}
      aria-haspopup="menu" aria-expanded={open} aria-controls={id} popoverTarget={id}>
      <ApprovalIcon mode={mode} />
    </button>
    <div id={id} className="approval-menu" popover="auto" role="menu" aria-label="Approval mode"
      onToggle={event => setOpen(event.newState === 'open')}
      onKeyDown={event => {
        const menu = event.currentTarget
        if (event.key === 'Tab') { menu.hidePopover(); return }
        const items = Array.from(menu.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'))
        const current = items.indexOf(document.activeElement as HTMLButtonElement)
        const next = event.key === 'ArrowDown' ? (current + 1) % items.length
          : event.key === 'ArrowUp' ? (current + items.length - 1) % items.length
          : event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : -1
        if (next >= 0) { event.preventDefault(); items[next].focus() }
      }}>
      {APPROVAL_MODES.map(option => {
        const reason = option.id === 'plan' ? planReason : undefined
        return <button key={option.id} type="button" role="menuitemradio" tabIndex={-1}
          aria-checked={mode === option.id} aria-disabled={Boolean(reason)} data-mode={option.id}
          ref={button => { if (mode === option.id) button?.setAttribute('autofocus', ''); else button?.removeAttribute('autofocus') }}
          onClick={event => {
            if (reason) return
            event.currentTarget.closest<HTMLElement>('[popover]')?.hidePopover()
            onChange(option.id)
          }}>
          <ApprovalIcon mode={option.id} />
          <span><strong>{option.label}</strong><small>{reason ?? option.detail}</small></span>
          {mode === option.id && <svg className="approval-check" width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><path d="m3 8 3 3 7-7" fill="none" stroke="currentColor" strokeWidth="1.5" /></svg>}
        </button>
      })}
    </div>
  </>
}
