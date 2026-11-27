import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'

import { errorText } from '../lib/ipc'
import type { Preferences } from '../lib/settings'
import {
  filterSkills, skillRoots, skillsSummary, withSkillEnabled, type Skill, type Skills,
} from '../lib/skills'
import { INJECTION_LABELS, setToolsEnabled, toolsCatalog, type Catalog } from '../lib/tools'
import { IntegrationsSheet } from './IntegrationsSheet'
import { Group, Toggle } from './SettingsRows'

/// One pane, two questions. Add-ons answers "what can this window offer a
/// harness" — the switch, the endpoint, the harnesses it reaches — and hands the
/// full list to the Manage sheet, which is where the four counted sections live
/// because 32 tool cards do not fit in a settings tab. Skills answers "what does
/// this workspace already know": a folder listing, each row with the switch that
/// decides whether a harness is handed that skill.
const TABS = ['Add-ons', 'Skills'] as const
type Tab = typeof TABS[number]

/// The catalogue, as the registry reports it.
///
/// Nothing here is written by hand: the rows, their availability per harness and
/// the switch all come from tools_catalog, so the pane cannot promise a tool the
/// MCP endpoint does not serve, and a harness that cannot be given the server is
/// listed with the reason instead of being left out.
///
/// `extras` is the Project checks form. It arrives from the settings view and
/// belongs to the add-ons half: it is a command this window runs after a turn,
/// not a skill a harness reads.
export function ToolsSettings({ extras, preferences, update, onChangeSection }: {
  extras?: ReactNode
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
  /// The sheet and its footer buttons move the reader to another section, which
  /// is the settings view's own navigation rather than a link to a document.
  onChangeSection: (section: 'Voice' | 'Connections') => void
}) {
  const [sheetOpen, setSheetOpen] = useState(false)
  const [tab, setTab] = useState<Tab>('Add-ons')
  const [catalog, setCatalog] = useState<Catalog | null>(null)
  const [status, setStatus] = useState('')
  const [busy, setBusy] = useState(false)

  const load = async () => {
    try {
      setCatalog(await toolsCatalog())
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  useEffect(() => {
    let stale = false
    void toolsCatalog().then(
      (catalog) => { if (!stale) setCatalog(catalog) },
      (err) => { if (!stale) setStatus(errorText(err)) },
    )
    return () => { stale = true }
  }, [])

  const toggle = async (enabled: boolean) => {
    setBusy(true)
    setStatus('')
    try {
      await setToolsEnabled(enabled)
      await load()
      setStatus(enabled
        ? 'Tools are on. New turns carry the shim; a running turn keeps the argv it started with.'
        : 'Tools are off. No harness is given the shim, and the endpoint refuses calls.')
    } catch (err) {
      setStatus(errorText(err))
    } finally {
      setBusy(false)
    }
  }

  const callable = catalog?.harnesses.filter((harness) => harness.callable) ?? []

  return (
    <>
      <nav className="settings-tabs" aria-label="Tools settings">
        {TABS.map((item) => (
          <button key={item} type="button" aria-pressed={tab === item}
            onClick={() => { setTab(item); setStatus('') }}>{item}</button>
        ))}
      </nav>

      {tab === 'Add-ons' && <>
        <Group title="Tools & integrations">
          <div className="settings-row">
            <span>
              <strong>Tools &amp; integrations</strong>
              <small>
                Every tool this window serves, the MCP servers each harness reports, the skills
                this workspace mounts, and the models Nova has downloaded.
              </small>
            </span>
            {/* toggle, not show: a second click on Manage has to dismiss the sheet,
                or the one control that opened it cannot close it. */}
            <button type="button" onClick={() => setSheetOpen(true)}>Manage</button>
          </div>
          <Toggle
            title="Offer the tool server to agents"
            checked={Boolean(catalog?.enabled)}
            disabled={!catalog || busy}
            onChange={(enabled) => void toggle(enabled)}
            detail={catalog?.endpoint
              ? 'Loopback endpoint: ' + catalog.endpoint + ' — a per-launch token, and no browser origin may call it.'
              : 'The endpoint has not started yet.'}
          />
          <p className="settings-help">
            {callable.length
              ? 'Desktop can attach the shim to ' + callable.map((harness) => harness.name).join(', ') + '.'
              : 'Reading harness compatibility…'}
          </p>
        </Group>

        <Group title="Harnesses">
          {/* The one row the table cannot derive: a harness's badge says what the
              app *intends* to do, this says the shim the intent depends on is
              missing, which is why a turn comes back with no tools. */}
          {catalog?.enabled && !catalog.shim && (
            <div className="settings-row">
              <span>
                <strong>No tool shim for this build</strong>
                <small>Harnesses get no Nova Desktop tools. Run npm run tools:shim.</small>
              </span>
            </div>
          )}
          {catalog?.harnesses.map((harness) => (
            <div className="settings-row" key={harness.id}>
              <span>
                <strong>{harness.name}</strong>
                <small>{harness.note ?? INJECTION_LABELS[harness.injection]}</small>
              </span>
              <span className={catalog.listedTools?.includes(harness.id) ? 'tools-badge is-on' : 'tools-badge'}>
                {catalog.activity?.some((call) => call.harness === harness.id && call.ok) ? 'Called successfully' : catalog.activity?.some((call) => call.harness === harness.id) ? 'Call failed' : catalog.listedTools?.includes(harness.id) ? 'Discovered tools' : harness.callable ? 'Configured' : 'Manual setup'}
              </span>
            </div>
          ))}
          {!catalog && <p className="settings-help">Loading the catalogue…</p>}
        </Group>

        <Group title="Recent Desktop tool calls">
          {catalog?.activity?.slice(-10).reverse().map((call, index) => <div className="settings-row" key={index}><span><strong>{call.name}</strong><small>{call.harness || "Unattributed client"} · {call.turnId.slice(0,8)} · {call.durationMs} ms</small></span><span className="tools-badge">{call.ok ? "Succeeded" : "Failed"}</span></div>)}
          {!catalog?.activity?.length && <p className="settings-help">No Desktop tool has been invoked this launch. Configuration and discovery alone do not verify a call.</p>}
          <button type="button" onClick={() => void load()}>Refresh activity</button>
        </Group>
        {sheetOpen && <IntegrationsSheet preferences={preferences} update={update} onClose={() => setSheetOpen(false)}
          onSection={(section) => { setSheetOpen(false); onChangeSection(section) }} />}

        {extras}
      </>}

      {tab === 'Skills' && <SkillsPane preferences={preferences} update={update} />}

      <p className="settings-status" role="status">{status}</p>
    </>
  )
}

/// The skills folders this window mounts, listed and searchable.
///
/// The list is the answer of tools_skills and nothing else: the filtering lives
/// in src/lib/skills.ts so node --test covers it, and this component only wires
/// the search box, the folder chips and the switches to it. Loading happens once
/// when the tab opens — a folder listing does not change while the pane is on
/// screen, and a poll would report edits the harnesses have already read.
///
/// A switch stores the directory name in preferences; the page pushes that list
/// to Rust (App.tsx), which stops reporting the skill to a harness. The row flips
/// immediately and the folder on disk is untouched.
function SkillsPane({ preferences, update }: {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
}) {
  const [skills, setSkills] = useState<Skills | null>(null)
  const [status, setStatus] = useState('')
  const [query, setQuery] = useState('')
  const [root, setRoot] = useState('')

  useEffect(() => {
    let stale = false
    void invoke<Skills>('tools_skills').then(
      (value) => { if (!stale) setSkills(value) },
      (err) => { if (!stale) setStatus(errorText(err)) },
    )
    return () => { stale = true }
  }, [])

  const all = skills?.skills ?? []
  const roots = skillRoots(all)
  const shown = filterSkills(all, { root, query })

  const flip = (skill: Skill, enabled: boolean) => {
    update({ disabledSkills: withSkillEnabled(preferences.disabledSkills, skill.directory, enabled) })
    setSkills((current) => current && {
      skills: current.skills.map((row) => row.directory === skill.directory ? { ...row, enabled } : row),
      count: current.count,
    })
  }

  return (
    <>
      <Group title="Skills">
        <label className="settings-row">
          <span>
            <strong>Skills this workspace mounts</strong>
            <small>
              Read from .nova/skills in this workspace, then ~/.nova, ~/.claude and
              ~/.agents. A project copy wins over a home copy of the same name.
            </small>
          </span>
          <input aria-label="Search skills" value={query} placeholder="Search skills"
            maxLength={80} onChange={(event) => setQuery(event.target.value)} />
        </label>
        <div className="skills-chips" role="group" aria-label="Skills by folder">
          <button type="button" aria-pressed={!root} onClick={() => setRoot('')}>All folders</button>
          {roots.map((label) => (
            <button key={label} type="button" aria-pressed={root === label}
              onClick={() => setRoot(label)}>{label}</button>
          ))}
        </div>
        <p className="settings-help">{skillsSummary(all.length, shown.length, roots.length)}</p>
        {shown.map((skill) => (
          <div className="settings-row" key={skill.root + '/' + skill.directory}>
            <span>
              <strong>{skill.name}</strong>
              <small>{skill.description || 'No description in SKILL.md.'}</small>
            </span>
            <span className="tools-badge">{skill.root}</span>
            <input {...{ switch: '' }} type="checkbox" role="switch" className="settings-switch"
              aria-label={'Hand ' + skill.name + ' to agents'} checked={skill.enabled}
              onChange={(event) => flip(skill, event.target.checked)} />
          </div>
        ))}
        {skills && !shown.length && (
          <p className="settings-help">
            {all.length
              ? 'Nothing matches that search or folder. Clear one of them to widen the list.'
              : 'No SKILL.md folder was found in any mounted folder yet.'}
          </p>
        )}
        {!skills && !status && <p className="settings-help">Reading the mounted folders…</p>}
      </Group>
      <p className="settings-status" role="status">{status}</p>
    </>
  )
}
