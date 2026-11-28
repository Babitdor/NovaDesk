import type { ButtonHTMLAttributes, ReactNode } from 'react'

/// Shared settings groups, toggles, and actions.

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


const ICONS = {
  save: 'M5 3h12l4 4v14H3V3h2Zm2 0v6h10V3M7 21v-8h10v8',
  refresh: 'M20 7v5h-5M4 17v-5h5M6 7a7 7 0 0 1 12-2l2 2M4 17l2 2a7 7 0 0 0 12-2',
  download: 'M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5',
  upload: 'M12 16V4m-5 5 5-5 5 5M4 16v5h16v-5',
  trash: 'M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7',
  copy: 'M8 8h13v13H8zM16 8V3H3v13h5',
  external: 'M14 3h7v7m0-7L11 13M11 4H4v16h16v-7',
  plus: 'M12 4v16M4 12h16',
  edit: 'm15 4 5 5M4 20l1-6L16 3l5 5-11 11-6 1Z',
  close: 'm6 6 12 12M18 6 6 18',
  check: 'm4 12 5 5L20 6',
  settings: 'M3 7h18M3 17h18M8 3v8m8 2v8',
  play: 'm8 4 12 8-12 8Z',
  stop: 'M5 5h14v14H5z',
  microphone: 'M9 5a3 3 0 0 1 6 0v7a3 3 0 0 1-6 0V5ZM5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8',
  file: 'M14 2H4v20h16V8l-6-6Zm0 0v6h6M8 13h8m-8 4h6',
  back: 'm10 5-7 7 7 7M3 12h18',
  sparkles: 'm12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5L12 3Z',
  palette: 'M12 3a9 9 0 1 0 0 18h2a2 2 0 0 0 0-4h-1a2 2 0 0 1 0-4h5a3 3 0 0 0 3-3 9 9 0 0 0-9-7ZM7 10h.01M10 6.5h.01M15 7h.01M6.5 15h.01',
  agents: 'M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M9 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8ZM17 4a4 4 0 0 1 0 7m5 10v-2a4 4 0 0 0-3-3.87',
  clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18ZM12 7v5l3 2',
  tools: 'm14 6 4 4 3-3a6 6 0 0 1-8 8l-7 7-4-4 7-7a6 6 0 0 1 8-8l-3 3Z',
  plug: 'M8 3v5m8-5v5M6 8h12v4a6 6 0 0 1-12 0V8Zm6 10v4',
  info: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18ZM12 11v6m0-10h.01',
} as const

export function SettingsIcon({ name }: { name: keyof typeof ICONS }) {
  return <svg className="settings-icon" width="18" height="18" viewBox="0 0 24 24"
    fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round"
    aria-hidden="true" focusable="false"><path d={ICONS[name]} /></svg>
}

export function ActionButton({ icon, children, className, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { icon: keyof typeof ICONS }) {
  return <button type="button" {...props} className={'settings-button' + (className ? ' ' + className : '')}>
    <SettingsIcon name={icon} />{children}
  </button>
}
