import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'

import { errorText } from '../lib/ipc'
import { filterSkills, skillRoots, skillsSummary, type Skills } from '../lib/skills'
import {
  GROUP_LABELS, INJECTION_LABELS, setToolsEnabled, toolsCatalog,
  type Catalog, type ToolGroup, type ToolRow,
} from '../lib/tools'
import { Group, Toggle } from './SettingsRows'

/// The order the groups are read in, which is the order of the work a task does:
/// start it, queue for it, look at the page, look at the code, look around.
const ORDER: ToolGroup[] = ['tasks', 'queue', 'browser', 'files', 'discovery']

/// One pane, two questions. Add-ons answers "what can this window offer a
/// harness" — the tool server, the harnesses it reaches, the catalogue, and the
/// app surface none of them reports. Skills answers "what does this workspace
/// already know", which is a folder listing rather than a switch: the harnesses
/// read SKILL.md themselves, so the tab lists them and does not pretend to turn
/// any of them on or off.
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
export function ToolsSettings({ extras }: { extras?: ReactNode }) {
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
  const rows = (group: ToolGroup) => (catalog?.tools ?? []).filter((tool) => tool.group === group)

  return (
    <>
      <nav className="settings-tabs" aria-label="Tools settings">
        {TABS.map((item) => (
          <button key={item} type="button" aria-pressed={tab === item}
            onClick={() => { setTab(item); setStatus('') }}>{item}</button>
        ))}
      </nav>

      {tab === 'Add-ons' && <>
        <Group title="Nova Desktop Tools">
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
              ? 'Turns started on ' + callable.map((harness) => harness.name).join(', ') + ' carry a stdio shim that speaks MCP back to this window.'
              : 'No installed harness can be given the shim this launch, so nothing can call these tools yet.'}
          </p>
        </Group>

        <Group title="Harnesses">
          {catalog?.harnesses.map((harness) => (
            <div className="settings-row" key={harness.id}>
              <span>
                <strong>{harness.name}</strong>
                <small>{harness.note ?? INJECTION_LABELS[harness.injection]}</small>
              </span>
              <span className={harness.callable ? 'tools-badge is-on' : 'tools-badge'}>
                {harness.callable ? 'Can call tools' : 'Not callable'}
              </span>
            </div>
          ))}
          {!catalog && <p className="settings-help">Loading the catalogue…</p>}
        </Group>

        {ORDER.map((group) => {
          const tools = rows(group)
          if (!tools.length) return null
          return (
            <Group title={GROUP_LABELS[group]} key={group}>
              {tools.map((tool) => <ToolCard key={tool.id} tool={tool} callable={callable.length} />)}
            </Group>
          )
        })}

        <Group title="Apps">
          <div className="settings-row">
            <span>
              <strong>Apps</strong>
              <small>
                None of the harnesses Nova drives reports an app surface, so this list
                is empty. It stays empty rather than listing something invented.
              </small>
            </span>
            <span className="tools-badge">Not reported</span>
          </div>
        </Group>

        {extras}
      </>}

      {tab === 'Skills' && <SkillsPane />}

      <p className="settings-status" role="status">{status}</p>
    </>
  )
}

/// The skills folders this window mounts, listed and searchable.
///
/// The list is the answer of tools_skills and nothing else: the filtering lives
/// in src/lib/skills.ts so node --test covers it, and this component only wires
/// the search box and the folder chips to it. Loading happens once when the tab
/// opens — a folder listing does not change while the pane is on screen, and a
/// poll would report edits the harnesses have already read.
function SkillsPane() {
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

function ToolCard({ tool, callable }: { tool: ToolRow; callable: number }) {
  return (
    <div className="settings-row tools-card">
      <span>
        <strong>{tool.title}</strong>
        <small>{tool.description}</small>
      </span>
      <span className="tools-badges">
        <span className="tools-badge">{tool.id}</span>
        <span className="tools-badge">{tool.handler === 'frontend' ? 'answered by the window' : 'answered by Rust'}</span>
        {tool.requiresWindow && <span className="tools-badge">needs the window</span>}
        {!callable && <span className="tools-badge">no harness can call it yet</span>}
      </span>
    </div>
  )
}
