import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

import { errorText } from '../lib/ipc'
import {
  GROUP_LABELS, INJECTION_LABELS, setToolsEnabled, toolsCatalog,
  type Catalog, type ToolGroup, type ToolRow,
} from '../lib/tools'
import { Group, Toggle } from './SettingsRows'

/// The order the groups are read in, which is the order of the work a task does:
/// start it, queue for it, look at the page, look at the code, look around.
const ORDER: ToolGroup[] = ['tasks', 'queue', 'browser', 'files', 'discovery']

type Skill = { name: string; directory: string; root: string; description: string }
type Skills = { skills: Skill[]; count: number }

/// The catalogue, as the registry reports it.
///
/// Nothing here is written by hand: the rows, their availability per harness and
/// the switch all come from tools_catalog, so the pane cannot promise a tool the
/// MCP endpoint does not serve, and a harness that cannot be given the server is
/// listed with the reason instead of being left out.
export function ToolsSettings() {
  const [catalog, setCatalog] = useState<Catalog | null>(null)
  const [skills, setSkills] = useState<Skills | null>(null)
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

  const loadSkills = async () => {
    setStatus('')
    try { setSkills(await invoke<Skills>('tools_skills')) }
    catch (err) { setStatus(errorText(err)) }
  }

  const callable = catalog?.harnesses.filter((harness) => harness.callable) ?? []
  const rows = (group: ToolGroup) => (catalog?.tools ?? []).filter((tool) => tool.group === group)

  return (
    <>
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

      <Group title="Skills and apps">
        <p className="settings-help">
          Skills are read from the folders Nova mounts (.nova/skills in this workspace,
          then ~/.nova, ~/.claude and ~/.agents). Apps are whatever the harnesses
          report — nothing is invented here.
        </p>
        <div className="settings-action">
          <button type="button" onClick={() => void loadSkills()}>List skills</button>
        </div>
        {skills && (
          <pre className="settings-output">{skills.count
            ? skills.skills.map((skill) => `${skill.name} — ${skill.description || 'no description'} (${skill.root})`).join('\n')
            : 'No skills found in the mounted folders.'}</pre>
        )}
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
