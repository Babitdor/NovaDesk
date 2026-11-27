import { useEffect, useRef, useState } from 'react'

import { clearAllDrafts, draftCount } from '../lib/drafts'
import type { WorkspaceInfo } from '../lib/events'
import {
  clearCrashReports, crashReports, errorText, openLauncher, setQuickLauncher, setWidget, type CrashReport,
} from '../lib/ipc'
import { clampInterfaceSize, INTERFACE_MAX, INTERFACE_MIN, type Preferences } from '../lib/settings'
import { markdownify } from '../lib/ipc'
import { Markdown } from './Markdown'
import { Group, Toggle } from './SettingsRows'
import { ShortcutInput } from './ShortcutInput'

type Props = {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
  workspace: WorkspaceInfo | null
}

/// Everything the General pane promises, backed by a preference or a native
/// call. A control that could not be made real is not in this list.
export function GeneralSettings({ preferences: p, update, workspace }: Props) {
  const [shortcut, setShortcut] = useState(p.quickLauncherShortcut)
  const [status, setStatus] = useState('')
  const [latest, setLatest] = useState<CrashReport | null>(null)
  const [dialog, setDialog] = useState(false)
  const [drafts, setDrafts] = useState(() => draftCount())
  const root = workspace?.root ?? ''

  const resize = (delta: number) => update({ interfaceSize: clampInterfaceSize(p.interfaceSize + delta) })

  /// Registration is the only step that can fail (a chord already owned by
  /// another application), so the toggle follows the platform, not the click.
  const toggleLauncher = async (enabled: boolean) => {
    setStatus('')
    try {
      await setQuickLauncher(enabled, enabled ? shortcut : '')
      update({ quickLauncher: enabled, quickLauncherShortcut: shortcut })
      setStatus(enabled ? 'Shortcut registered.' : 'Shortcut released.')
    } catch (err) {
      update({ quickLauncher: false })
      setStatus(errorText(err))
    }
  }
  const saveShortcut = async () => {
    setStatus('')
    try {
      await setQuickLauncher(p.quickLauncher, shortcut)
      update({ quickLauncherShortcut: shortcut })
      setStatus('Shortcut saved.')
    } catch (err) { setStatus(errorText(err)) }
  }
  const toggleWidget = async (enabled: boolean) => {
    setStatus('')
    try {
      await setWidget(enabled, p.widgetPinned)
      update({ widgetEnabled: enabled })
    } catch (err) { setStatus(errorText(err)) }
  }
  const pinWidget = async (pinned: boolean) => {
    setStatus('')
    try {
      await setWidget(p.widgetEnabled, pinned)
      update({ widgetPinned: pinned })
    } catch (err) { setStatus(errorText(err)) }
  }
  const showReports = async () => {
    setStatus('')
    try {
      const reports = await crashReports()
      if (!reports.length) {
        setStatus('No crash reports have been recorded.')
        return
      }
      setLatest(reports[0])
      setDialog(true)
    } catch (err) { setStatus(errorText(err)) }
  }

  return (
    <>
      <Group title="Interface">
        <div className="settings-row">
          <span><strong>Interface size</strong>
            <small>Scales the whole window. Ctrl + and Ctrl − also work, Ctrl 0 resets.</small></span>
          <span className="settings-stepper">
            <button type="button" aria-label="Smaller interface" disabled={p.interfaceSize <= INTERFACE_MIN}
              onClick={() => resize(-10)}>A−</button>
            <output aria-live="polite">{p.interfaceSize}%</output>
            <button type="button" aria-label="Larger interface" disabled={p.interfaceSize >= INTERFACE_MAX}
              onClick={() => resize(10)}>A+</button>
            <button type="button" disabled={p.interfaceSize === 100} onClick={() => update({ interfaceSize: 100 })}>Reset</button>
          </span>
        </div>
        <Toggle title="Compact conversation"
          detail="Tighter transcript spacing, so more of a long task is on screen."
          checked={p.compactConversation} onChange={(compactConversation) => update({ compactConversation })} />
        <Toggle title="Expand tool activity"
          detail="Tool calls and their output open by default instead of waiting for a click."
          checked={p.expandToolActivity} onChange={(expandToolActivity) => update({ expandToolActivity })} />
      </Group>

      <Group title="Quick launcher">
        <Toggle title="Global quick launcher"
          detail="One chord opens a small prompt window from any application."
          checked={p.quickLauncher} onChange={(enabled) => void toggleLauncher(enabled)} />
        <div className="settings-row">
          <span><strong>Shortcut</strong><small>Record a chord, then save it. Escape cancels.</small></span>
          <span className="settings-row-controls">
            <ShortcutInput value={shortcut} onChange={setShortcut} />
            <button type="button" disabled={!shortcut || shortcut === p.quickLauncherShortcut}
              onClick={() => void saveShortcut()}>Save</button>
            <button type="button" onClick={() => void openLauncher().catch((err) => setStatus(errorText(err)))}>
              Open
            </button>
          </span>
        </div>
        <p className="settings-help">
          The chord is registered with the operating system, so it fires while another window has focus.
          Enter in the launcher starts the task in the main window; Escape closes it.
          If the chord is already taken, the platform says so here and the old one stays off.
        </p>
      </Group>

      <Group title="Running agents widget">
        <Toggle title="Show what agents are doing"
          detail="A small always-on-top window lists running turns while the main window is in the background."
          checked={p.widgetEnabled} onChange={(enabled) => void toggleWidget(enabled)} />
        <Toggle title="Keep the widget on top" disabled={!p.widgetEnabled}
          detail="Pinned windows stay above other applications."
          checked={p.widgetPinned} onChange={(pinned) => void pinWidget(pinned)} />
        <p className="settings-help">
          The widget reads the same run list as the composer and never starts or stops anything.
          Clicking a row brings Nova back to that task.
        </p>
      </Group>

      <Group title="Crash recovery">
        <Toggle title="Restart automatically after a crash"
          detail="Reopens Nova after a native crash, up to three times in fifteen minutes."
          checked={p.restartAfterCrash} onChange={(restartAfterCrash) => update({ restartAfterCrash })} />
        <Toggle title="Crash reports"
          detail="Write a report next to the app data, including errors reported by the page."
          checked={p.crashReports} onChange={(enabled) => update({ crashReports: enabled })} />
        <div className="settings-action">
          <button type="button" onClick={() => void showReports()}>Show latest crash report</button>
          <button type="button" onClick={() => void clearCrashReports().then(
            () => { setLatest(null); setStatus('Crash reports cleared.') },
            (err) => setStatus(errorText(err)))}>Clear reports</button>
        </div>
        <p className="settings-help">
          Auto-restart covers a panic in Nova itself and errors reported by the page. A killed process,
          power loss or a fault inside a graphics driver cannot be recovered from inside the process.
        </p>
      </Group>

      <Group title="Unsent drafts">
        <Toggle title="Keep unsent drafts"
          detail="Composer text survives a task switch and a restart for 30 days."
          checked={p.keepDrafts} onChange={(keepDrafts) => {
            update({ keepDrafts })
            if (!keepDrafts) { clearAllDrafts(); setDrafts(0) }
          }} />
        <div className="settings-row">
          <span><strong>Saved drafts</strong><small>{drafts ? `${drafts} saved` : 'Nothing saved yet.'}</small></span>
          <button type="button" disabled={!drafts} onClick={() => { clearAllDrafts(); setDrafts(0); setStatus('Saved drafts cleared.') }}>
            Clear saved drafts
          </button>
        </div>
      </Group>

      <Group title="Projects">
        {p.projects.length === 0 && <p className="settings-help">Folders you open appear here.</p>}
        {p.projects.map((project) => (
          <div key={project.root} className="settings-row">
            <span><strong>{project.name}</strong><small>{project.root}</small></span>
            <button type="button" onClick={() => update({
              projects: p.projects.filter(({ root: entry }) => entry !== project.root),
            })}>Remove</button>
          </div>
        ))}
        <p className="settings-help">
          Remove forgets the entry. It never deletes the folder or withdraws workspace trust.
        </p>
      </Group>

      <Group title="Workspace behavior">
        <Toggle title="Reopen last folder" checked={p.restoreFolder} onChange={(restoreFolder) => update({ restoreFolder })} />
        <Toggle title="Render Markdown in chat" checked={p.markdown} onChange={(markdown) => update({ markdown })} />
        {root && <Toggle title="Allow Nova to trust this folder on startup"
          detail="Nova saves the folder approval in its CLI configuration. Revoke it with nova paths revoke."
          checked={p.trustedWorkspaces.includes(root)} onChange={(enabled) => update({
            trustedWorkspaces: enabled ? [...p.trustedWorkspaces, root]
              : p.trustedWorkspaces.filter((path) => path !== root),
          })} />}
      </Group>

      {/* Markdownify lives here rather than in Tools: it converts a paste the
          user made, and the Tools pane is about what a harness can call. */}
      <Markdownify />

      <p className="settings-status" role="status">{status}</p>

      {dialog && latest && <CrashDialog report={latest} onClose={() => setDialog(false)}
        onClear={() => void clearCrashReports().then(
          () => { setLatest(null); setDialog(false); setStatus('Crash reports cleared.') },
          (err) => setStatus(errorText(err)))} />}
    </>
  )
}

/// The HTML to Markdown converter, unchanged: it was in Tools only because that
/// section had nowhere better to put it.
function Markdownify() {
  const [html, setHtml] = useState('')
  const [converted, setConverted] = useState('')
  const [converting, setConverting] = useState(false)
  const [failure, setFailure] = useState('')

  return (
    <Group title="Markdownify">
      <form className="settings-convert" aria-label="Convert HTML to Markdown" onSubmit={async (event) => {
        event.preventDefault()
        setConverting(true)
        setFailure('')
        try { setConverted(await markdownify(html)) }
        catch (err) { setFailure(errorText(err)) }
        finally { setConverting(false) }
      }}>
        <label>HTML<textarea value={html} onChange={(event) => setHtml(event.target.value)} rows={5} required
          placeholder="<h1>Hello</h1>" maxLength={512 * 1024} /></label>
        <div className="settings-action"><button type="submit" disabled={converting}>
          {converting ? 'Converting…' : 'Convert to Markdown'}
        </button></div>
        {failure && <p className="settings-help">{failure}</p>}
        {converted && <>
          <label>Markdown<textarea value={converted} readOnly rows={5} /></label>
          <Markdown text={converted} />
        </>}
      </form>
    </Group>
  )
}

function CrashDialog({ report, onClose, onClear }: {
  report: CrashReport; onClose: () => void; onClear: () => void
}) {
  const [copy, setCopy] = useState('')
  const dialog = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const modal = dialog.current
    modal?.showModal()
    return () => { modal?.close() }
  }, [])
  return (
    <dialog ref={dialog} className="crash-dialog" {...{ closedby:'any' }} onClose={(event) => {
        // Effect replay can deliver close after the dialog has reopened.
        if (!event.currentTarget.open) onClose()
      }} aria-label="Crash report">
      <div className="crash-card">
        <header>
          <h2>{report.source === 'webview' ? 'Page error' : 'Crash'} · {new Date(report.at).toLocaleString()}</h2>
          <button type="button" className="ghost" onClick={onClose} aria-label="Close crash report">×</button>
        </header>
        <p className="crash-message">{report.message}</p>
        {report.location && <pre className="crash-location">{report.location}</pre>}
        {report.backtrace && <pre className="crash-backtrace">{report.backtrace}</pre>}
        <div className="settings-action">
          <button type="button" onClick={() => {
            void navigator.clipboard?.writeText(report.path)
              .then(() => setCopy('Copied'), () => setCopy('Copy failed'))
          }}>{copy || 'Copy path'}</button>
          <button type="button" onClick={onClear}>Clear reports</button>
          <button type="button" onClick={onClose}>Close</button>
        </div>
      </div>
    </dialog>
  )
}
