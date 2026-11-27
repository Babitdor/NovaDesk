import { useEffect, useState } from 'react'
import type { ReactNode } from 'react'
import { invoke } from '@tauri-apps/api/core'

import { errorText } from '../lib/ipc'
import { appRows, sectionCounts, type AppRow } from '../lib/integrations'
import { withSkillEnabled, type Skill, type Skills } from '../lib/skills'
import {
  GROUP_LABELS, mcpServers, toolsCatalog,
  type Catalog, type McpDiscovery, type ToolGroup,
} from '../lib/tools'
import type { Preferences } from '../lib/settings'
import { speakCatalog, speakInstalled, speakRuntimeReady } from '../lib/speak'
import { voiceCatalog, voiceInstalled } from '../lib/voice'

/// The order the tool groups are read in, which is the order of the work a task
/// does: start it, queue for it, look at the page, look at the code, look around.
const ORDER: ToolGroup[] = ['tasks', 'queue', 'browser', 'files', 'discovery']

type Key = 'tools' | 'servers' | 'skills' | 'apps'

/// Settings -> Tools -> Manage: one sheet with four counted sections.
///
/// Every row is an answer Nova already has. The tools are the registry, the
/// servers are what each harness's own CLI reported just now, the skills are the
/// folders this workspace mounts, and the apps are the downloads on disk. The
/// sheet adds no list of its own, so it cannot claim something the app cannot do.
///
/// Inventory loads after the reader selects Manage.
export function IntegrationsSheet({ preferences, update, onSection }: {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
  onSection: (section: 'Voice' | 'Connections') => void
}) {
  const [catalog, setCatalog] = useState<Catalog | null>(null)
  const [skills, setSkills] = useState<Skills | null>(null)
  const [servers, setServers] = useState<McpDiscovery | null>(null)
  const [apps, setApps] = useState<AppRow[] | null>(null)
  const [open, setOpen] = useState<Record<Key, boolean>>({ tools: false, servers: false, skills: false, apps: false })
  const [status, setStatus] = useState('')
  const [hasOpened, setHasOpened] = useState(false)

  const loadTools = async () => {
    try {
      setCatalog(await toolsCatalog())
      setSkills(await invoke<Skills>('tools_skills'))
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  const loadServers = async () => {
    try {
      setServers(await mcpServers())
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  const loadApps = async () => {
    try {
      const [voice, haveVoice, speak, haveSpeak, runtime] = await Promise.all([
        voiceCatalog(), voiceInstalled(), speakCatalog(), speakInstalled(), speakRuntimeReady(),
      ])
      setApps(appRows(voice, haveVoice, speak, haveSpeak, runtime))
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  // Everything starts when the sheet opens rather than when Settings does: the
  // sheet is an explicit "show me", and its four headings are counts, so a
  // section that has not been asked yet would otherwise read 0 — the one answer
  // this pane must never give. The CLI run is the slow one (~12 s here), so the
  // count stays a placeholder until it answers instead of pretending to be zero.
  useEffect(() => {
    if (!hasOpened) return
    let stale = false
    void toolsCatalog().then(
      (value) => { if (!stale) setCatalog(value) },
      (err) => { if (!stale) setStatus(errorText(err)) },
    )
    void invoke<Skills>('tools_skills').then(
      (value) => { if (!stale) setSkills(value) },
      (err) => { if (!stale) setStatus(errorText(err)) },
    )
    void mcpServers().then(
      (value) => { if (!stale) setServers(value) },
      (err) => { if (!stale) setStatus(errorText(err)) },
    )
    void Promise.all([
      voiceCatalog(), voiceInstalled(), speakCatalog(), speakInstalled(), speakRuntimeReady(),
    ]).then(
      ([voice, have, speak, haveSpeak, runtime]) => {
        if (!stale) setApps(appRows(voice, have, speak, haveSpeak, runtime))
      },
      (err) => { if (!stale) setStatus(errorText(err)) },
    )
    return () => { stale = true }
  }, [hasOpened])

  const openSection = (key: Key) => {
    setOpen((current) => ({ ...current, [key]: !current[key] }))
  }

  const refresh = async () => {
    setStatus('')
    setServers(null)
    setApps(null)
    await loadTools()
    await Promise.all([loadServers(), loadApps()])
  }

  /// The switch writes the directory name, not an index: the list Rust reads and
  /// the list preferences hold have to name the same folder.
  const toggleSkill = (skill: Skill, enabled: boolean) => {
    update({ disabledSkills: withSkillEnabled(preferences.disabledSkills, skill.directory, enabled) })
    setSkills((current) => current && {
      skills: current.skills.map((row) => row.directory === skill.directory ? { ...row, enabled } : row),
      count: current.count,
    })
  }

  const counts = sectionCounts({
    tools: catalog?.tools.length ?? 0,
    servers: servers?.servers.length ?? 0,
    skills: skills?.skills ?? [],
    apps: apps?.length ?? 0,
  })

  /// Counts stay unknown until their requests finish.
  const known: Record<Key, boolean> = {
    tools: Boolean(catalog), servers: Boolean(servers), skills: Boolean(skills), apps: Boolean(apps),
  }

  /// A heading number is a placeholder until its answer exists: a zero is a
  /// claim, and "not asked yet" is not a count of zero.
  const section = (key: Key, title: string, count: number | string, body: ReactNode) => (
    <section className="integrations-section">
      <button type="button" className="integrations-toggle" aria-expanded={open[key]}
        aria-controls={'nova-integrations-' + key}
        onClick={() => openSection(key)}>
        <span>{title}</span>
        <span className="integrations-count">{known[key] ? count : '\u2026'}</span>
      </button>
      <div id={'nova-integrations-' + key} className="integrations-body" hidden={!open[key]}>{body}</div>
    </section>
  )

  return (
    <div id="nova-integrations" popover="auto" className="notices-sheet integrations-sheet"
      onToggle={(event) => { if (event.newState === 'open') setHasOpened(true) }}
      role="dialog" aria-label="Tools and integrations">
      <header className="notices-head">
        <h2>Tools &amp; integrations</h2>
        <button type="button" popoverTarget="nova-integrations" popoverTargetAction="hide">Close</button>
      </header>

      <div className="integrations-scroll">
        <p className="integrations-intro">
          Nova can hand a harness a tool server, the harnesses keep their own MCP servers, this
          workspace mounts skill folders, and Nova downloads a few models of its own. All four are
          listed here as they are, not as a plan.
        </p>

        <div className="integrations-addon">
          <span>
            <strong>On-device voice</strong>
            <small>
              Download a Whisper or Breeze model to dictate without a service, or a voice to read
              replies aloud. Nothing is sent anywhere; the models run on this machine.
            </small>
          </span>
          <button type="button" onClick={() => onSection('Voice')}>Set up</button>
        </div>

        {section('tools', 'Nova tools', counts.tools, (
          <>
            <p className="integrations-note">
              {catalog
                ? catalog.tools.length + ' tools this window serves over one loopback endpoint, '
                  + (catalog.enabled ? 'offered to new turns.' : 'switched off in Add-ons.')
                : 'Reading the registry…'}
            </p>
            {ORDER.map((group) => {
              const rows = (catalog?.tools ?? []).filter((tool) => tool.group === group)
              if (!rows.length) return null
              return (
                <div key={group} className="integrations-group">
                  <h3>{GROUP_LABELS[group]}</h3>
                  {rows.map((tool) => (
                    <div className="integrations-row" key={tool.id}>
                      <span><strong>{tool.title}</strong><small>{tool.description}</small></span>
                      <code>{tool.id}</code>
                    </div>
                  ))}
                </div>
              )
            })}
          </>
        ))}

        {section('servers', 'MCP servers', counts.servers, (
          <>
            {servers?.servers.map((server) => (
              <div className="integrations-row" key={server.harnessId + ':' + server.name}>
                <span><strong>{server.name}</strong><small>{server.detail || server.harness}</small></span>
                <code>{server.harness}</code>
              </div>
            ))}
            {servers && !servers.servers.length && (
              <p className="integrations-note">
                None of the harness CLIs this window knows reports an MCP server for this workspace.
              </p>
            )}
            {!!servers?.timedOut.length && (
              <p className="integrations-note">
                {servers.timedOut.join(' and ')} did not answer before the discovery timeout, so
                their servers are unknown rather than absent.
              </p>
            )}
            {!!servers?.unsupported.length && (
              <p className="integrations-note">
                {servers.unsupported.join(' and ')} could not be asked: the CLI is missing, or it
                has no <code>mcp list</code>, so those servers are missing from this list.
              </p>
            )}
            {!servers && !status && <p className="integrations-note">Asking each harness…</p>}
          </>
        ))}

        {section('skills', 'Skills', counts.skills, (
          <>
            <p className="integrations-note">
              {counts.disabled
                ? counts.disabled + ' switched off: still listed here, and left out of what a harness is handed.'
                : 'Every skill in the mounted folders is handed to agents.'}
            </p>
            {(skills?.skills ?? []).map((skill) => (
              <div className="integrations-row" key={skill.root + '/' + skill.directory}>
                <span>
                  <strong>{skill.name}</strong>
                  <small>{skill.description || 'No description in SKILL.md.'}</small>
                </span>
                <code>{skill.root}</code>
                <input {...{ switch: '' }} type="checkbox" role="switch" className="settings-switch"
                  aria-label={'Hand ' + skill.name + ' to agents'} checked={skill.enabled}
                  onChange={(event) => toggleSkill(skill, event.target.checked)} />
              </div>
            ))}
            {!skills && !status && <p className="integrations-note">Reading the mounted folders…</p>}
          </>
        ))}

        {section('apps', 'Apps', counts.apps, (
          <>
            {apps?.map((app) => (
              <div className="integrations-row" key={app.id}>
                <span><strong>{app.name}</strong><small>{app.detail}</small></span>
              </div>
            ))}
            {apps && !apps.length && (
              <p className="integrations-note">
                Nothing downloaded yet. Voice and speech models are the only downloads Nova manages,
                and Set up above is where they come from.
              </p>
            )}
            {!apps && !status && <p className="integrations-note">Checking what is on disk…</p>}
          </>
        ))}

        <p className="integrations-note">
          A skill switch changes what Nova reports and hands to a harness; it does not delete a
          SKILL.md. MCP servers come from each harness's own configuration and are read here, never
          written.
        </p>
      </div>

      <footer className="integrations-foot">
        <button type="button" onClick={() => onSection('Connections')}>MCP setup</button>
        <span className="settings-status" role="status">{status}</span>
        <button type="button" onClick={() => void refresh()}>Refresh</button>
        <button type="button" popoverTarget="nova-integrations" popoverTargetAction="hide">Done</button>
      </footer>
    </div>
  )
}
