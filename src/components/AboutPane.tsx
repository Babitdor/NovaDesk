import { useEffect, useEffectEvent, useState } from 'react'

import { NOTICES } from '../data/notices'
import { appReport, checkAppUpdate, errorText, openLink, type AppReport, type UpdateCheck } from '../lib/ipc'
import type { Preferences } from '../lib/settings'

const REPOSITORY = 'https://github.com/Babitdor/NovaDesk'
const RELEASES = `${REPOSITORY}/releases`
const ISSUES = `${REPOSITORY}/issues`
/// Counters change from outside this pane (the dock opens, a turn starts) and
/// there is no event bus for "how many", so the pane samples them while open.
const REFRESH_MS = 4000

const UPDATE_MODES = [
  ['automatic', 'Automatic'],
  ['manual', 'Manual'],
  ['off', 'Off'],
] as const

type Props = {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
}

function formatBytes(bytes: number | null): string {
  if (bytes === null) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1 }
  // One decimal below 1,000 so a row whose value really moves (the local
  // database grows by bytes per task) shows that it moved.
  const shown = unit > 0 && value < 1000 ? value.toFixed(1) : Math.round(value).toString()
  return `${shown} ${units[unit]}`
}

function formatUptime(seconds: number): string {
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (hours) return `${hours}h ${String(minutes).padStart(2, '0')}m`
  if (minutes) return `${minutes}m`
  return `${Math.max(0, Math.floor(seconds))}s`
}

function formatAge(since: number): string {
  if (!since) return 'never'
  const seconds = Math.max(0, Math.round((Date.now() - since) / 1000))
  if (seconds < 45) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  return `${Math.round(hours / 24)}d ago`
}

/// localStorage stores UTF-16, so the byte count is twice the code-unit count —
/// which is why the row is labelled "≈". It is the honest order of magnitude,
/// not a file size on disk.
function localDatabaseBytes(): number {
  let bytes = 0
  for (let index = 0; index < localStorage.length; index += 1) {
    const key = localStorage.key(index) ?? ''
    bytes += (key.length + (localStorage.getItem(key)?.length ?? 0)) * 2
  }
  return bytes
}

export function AboutPane({ preferences, update }: Props) {
  const [report, setReport] = useState<AppReport | null>(null)
  const [reportError, setReportError] = useState('')
  const [database, setDatabase] = useState(0)
  const [checking, setChecking] = useState(false)
  const [checked, setChecked] = useState<UpdateCheck | null>(null)
  const [checkError, setCheckError] = useState('')
  const [details, setDetails] = useState(false)
  const [copied, setCopied] = useState('')
  const [filter, setFilter] = useState('')

  const refresh = async () => {
    try {
      setReport(await appReport())
      setDatabase(localDatabaseBytes())
      setReportError('')
    } catch (err) { setReportError(errorText(err)) }
  }

  const runCheck = async () => {
    setChecking(true)
    setCheckError('')
    try {
      const next = await checkAppUpdate()
      setChecked(next)
      update({ updateCheckedAt: Date.now() })
    } catch (err) {
      setChecked(null)
      setCheckError(errorText(err))
    } finally { setChecking(false) }
  }

  // The counters move from outside this pane (the dock opens, a turn starts) and
  // nothing emits an event for "how many", so the pane samples them while open.
  // Effect events keep the timers wired once while still reading today's mode.
  const open = useEffectEvent(() => {
    void refresh()
    if (preferences.updateMode !== 'off') void runCheck()
  })
  const tick = useEffectEvent(() => { void refresh() })

  useEffect(() => {
    // The first sample rides the same task queue as the polls, so this effect
    // only wires timers up and never sets state itself.
    const first = window.setTimeout(open, 0)
    const timer = window.setInterval(tick, REFRESH_MS)
    return () => { window.clearTimeout(first); window.clearInterval(timer) }
  }, [])

  const support = [
    `Nova Desktop ${report?.version ?? 'unknown'}`,
    `Identifier: ${report?.identifier ?? 'unknown'}`,
    `Platform: ${report ? `${report.os} ${report.arch}` : 'unknown'}`,
    `Data directory: ${report?.dataDir ?? 'unknown'}`,
    `Uptime: ${formatUptime(report?.uptimeSecs ?? 0)}`,
    `WebView: ${navigator.userAgent}`,
  ].join('\n')

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(support)
      setCopied('Copied')
    } catch {
      setCopied('Press Ctrl+C — the details are selected')
      const node = document.getElementById('nova-support-details')
      if (node) window.getSelection()?.selectAllChildren(node)
    }
  }

  // Check state, in the order a reader asks about it: is it on, is it running,
  // did it fail, and only then what the answer was.
  let updateState = 'Not checked'
  if (preferences.updateMode === 'off') updateState = 'Off'
  else if (checking) updateState = 'Checking…'
  else if (checkError) updateState = checkError
  else if (checked) updateState = checked.available
    ? `v${checked.latest} available · checked ${formatAge(preferences.updateCheckedAt)}`
    : `Up to date · checked ${formatAge(preferences.updateCheckedAt)}`

  const query = filter.trim().toLowerCase()
  const matches = query
    ? NOTICES.filter((notice) => `${notice.name} ${notice.version} ${notice.license}`.toLowerCase().includes(query))
    : NOTICES
  const groups = new Map<string, typeof NOTICES>()
  for (const notice of [...matches].sort((a, b) => (a.license === 'unknown' ? 1 : 0) - (b.license === 'unknown' ? 1 : 0)
    || a.license.localeCompare(b.license) || a.name.localeCompare(b.name))) {
    groups.set(notice.license, [...(groups.get(notice.license) ?? []), notice])
  }
  const unknown = NOTICES.filter(({ license }) => license === 'unknown').length

  return (
    <>
      <section className="settings-group">
        <h2>About Nova Desktop</h2>
        <div className="settings-row">
          <span><strong>Version</strong><small>Reported by the running build.</small></span>
          <span className="about-value">{report?.version ?? '—'}</span>
        </div>
        <div className="settings-row">
          <span><strong>License</strong><small>This repository declares no licence, so it grants no rights beyond the host's terms.</small></span>
          <button type="button" onClick={() => void openLink(REPOSITORY)}>Repository</button>
        </div>
        <div className="settings-row">
          <span><strong>Third-party notices</strong><small>{NOTICES.length} packages from this build's lockfiles{unknown ? `, ${unknown} without a declared licence` : ''}.</small></span>
          <button type="button" {...{ popovertarget: 'nova-notices' }}>View notices</button>
        </div>
        <div className="settings-row">
          <span><strong>Updates</strong><small>{updateState}</small></span>
          <span className="about-buttons">
            <button type="button" disabled={checking || preferences.updateMode === 'off'}
              onClick={() => void runCheck()}>{checking ? 'Checking…' : 'Check now'}</button>
            <button type="button" onClick={() => void openLink(checked?.available && checked.url ? checked.url : RELEASES)}>
              {checked?.available ? `v${checked.latest}` : 'Releases'}
            </button>
          </span>
        </div>
        <label className="settings-row">
          <span>
            <strong>Update mode</strong>
            <small>Nova has no installer: it checks GitHub and links to the release. It never installs or restarts itself.</small>
          </span>
          <select value={preferences.updateMode}
            onChange={(event) => update({ updateMode: event.target.value as Preferences['updateMode'] })}>
            {UPDATE_MODES.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
          </select>
        </label>
        <div className="settings-row">
          <span><strong>Source code</strong><small>Issues and releases live on GitHub.</small></span>
          <span className="about-buttons">
            <button type="button" onClick={() => void openLink(REPOSITORY)}>GitHub repository</button>
            <button type="button" onClick={() => void openLink(ISSUES)}>Issues</button>
          </span>
        </div>
      </section>

      <section className="settings-group">
        <h2>Resource usage</h2>
        <div className="settings-row">
          <span><strong>Native core memory</strong><small>Resident memory of this process, measured now.</small></span>
          <span className="about-value">{report ? formatBytes(report.rssBytes) : '—'}</span>
        </div>
        <div className="settings-row">
          <span><strong>Active agents</strong><small>Turns running right now.</small></span>
          <span className="about-value">{report?.agents ?? '—'}</span>
        </div>
        <div className="settings-row">
          <span><strong>Open terminals</strong><small>Shells the dock still owns.</small></span>
          <span className="about-value">{report?.terminals ?? '—'}</span>
        </div>
        <div className="settings-row">
          <span><strong>Local database</strong><small>≈ UTF-16 size of the settings and transcript keys in this window.</small></span>
          <span className="about-value">{formatBytes(database)}</span>
        </div>
        <div className="settings-action">
          <button type="button" onClick={() => void refresh()}>Refresh</button>
          {report && <small className="about-note">
            Uptime {formatUptime(report.uptimeSecs)}. The WebView and agent child processes run separately and are not counted here.
          </small>}
        </div>
        {reportError && <p className="settings-help" role="status">Could not read the counters: {reportError}</p>}
      </section>

      <section className="settings-group">
        <h2>Help &amp; support</h2>
        <div className="settings-row">
          <span><strong>Report a problem</strong><small>Open an issue with the details below.</small></span>
          <button type="button" onClick={() => void openLink(ISSUES)}>Open issues</button>
        </div>
        <div className="settings-row">
          <span><strong>App details for support</strong><small>Paste this with a bug report.</small></span>
          <span className="about-buttons">
            <button type="button" aria-expanded={details} onClick={() => setDetails(!details)}>
              {details ? 'Hide details' : 'Show details'}
            </button>
            <button type="button" onClick={() => void copy()}>Copy</button>
          </span>
        </div>
        {details && <pre id="nova-support-details" className="about-details">{support}</pre>}
        {copied && <p className="settings-help" role="status">{copied}</p>}
      </section>

      <div id="nova-notices" {...{ popover: 'auto' }} className="notices-sheet" role="dialog" aria-label="Third-party notices">
        <header className="notices-head">
          <h2>Third-party notices</h2>
          <input aria-label="Filter notices" placeholder="Filter by name or licence" value={filter}
            onChange={(event) => setFilter(event.target.value)} />
          <button type="button" {...{ popovertarget: 'nova-notices', popovertargetaction: 'hide' }}>Close</button>
        </header>
        <p className="notices-summary">
          {matches.length} of {NOTICES.length} packages · generated from this build&apos;s lockfiles by <code>npm run notices</code>
        </p>
        <div className="notices-list">
          {[...groups].map(([license, entries]) => (
            <section key={license}>
              <h3>
                {license === 'unknown' ? 'Not declared' : license}
                <span>{entries.length}</span>
              </h3>
              <ul>
                {entries.map(({ name, version, kind }) => (
                  <li key={`${kind}:${name}@${version}`}><span>{name}</span><code>{version}</code></li>
                ))}
              </ul>
            </section>
          ))}
          {!matches.length && <p className="settings-help">Nothing matches “{filter}”.</p>}
        </div>
      </div>
    </>
  )
}
