import { navigateComposerMenu } from '../lib/menu'
import { useEffect, useId, useRef, useState, type ReactNode } from 'react'

type Option<Value extends string> = { id: Value; label: string; detail: string; icon: ReactNode; reason?: string }

export function ComposerPicker<Value extends string>({ name, label, value, options, onChange, disabled = false }: {
  name: 'approval' | 'personality'; label: string; value: Value; options: Option<Value>[]
  onChange: (value: Value) => void; disabled?: boolean
}) {
  const id = useId()
  const panel = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const selected = options.find(option => option.id === value)!
  useEffect(() => {
    if (disabled && panel.current?.matches(':popover-open')) panel.current.hidePopover()
  }, [disabled])
  return <>
    <button type="button" className={`ghost composer-picker-trigger ${name}-trigger`} data-mode={value}
      aria-label={`${label}: ${selected.label}`} title={`${selected.label}: ${selected.detail}`} disabled={disabled}
      aria-haspopup="menu" aria-expanded={open} aria-controls={id} popoverTarget={id}>
      {selected.icon}
    </button>
    <div ref={panel} id={id} className={`composer-menu ${name}-menu`} popover="auto" role="menu" aria-label={label}
      onToggle={event => setOpen(event.newState === 'open')}
      onKeyDown={navigateComposerMenu}>
      {options.map(option => <button key={option.id} type="button" role="menuitemradio" tabIndex={-1}
        aria-checked={value === option.id} aria-disabled={Boolean(option.reason)} data-mode={option.id}
        ref={button => { if (value === option.id) button?.setAttribute('autofocus', ''); else button?.removeAttribute('autofocus') }}
        onClick={event => {
          if (option.reason || disabled) return
          event.currentTarget.closest<HTMLElement>('[popover]')?.hidePopover()
          onChange(option.id)
        }}>
        {option.icon}
        <span><strong>{option.label}</strong><small>{option.reason ?? option.detail}</small></span>
        {value === option.id && <svg className="composer-picker-check" width="14" height="14" viewBox="0 0 16 16" aria-hidden="true"><path d="m3 8 3 3 7-7" fill="none" stroke="currentColor" strokeWidth="1.5" /></svg>}
      </button>)}
    </div>
  </>
}
