import type { ReactNode } from 'react'

/// The two shapes every settings pane is built from. They live in their own file
/// so a pane can be split out without importing the whole Settings view back.

export function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="settings-group">
      {title && <h2>{title}</h2>}
      {children}
    </section>
  )
}

export function Toggle({ title, detail, checked, onChange, disabled }: {
  title: string; detail?: string; checked: boolean; onChange: (value: boolean) => void; disabled?: boolean
}) {
  return (
    <label className="settings-row">
      <span><strong>{title}</strong>{detail && <small>{detail}</small>}</span>
      <input {...{ switch: '' }} className="settings-switch" type="checkbox" role="switch"
        checked={checked} disabled={disabled} onChange={(event) => onChange(event.target.checked)} />
    </label>
  )
}
