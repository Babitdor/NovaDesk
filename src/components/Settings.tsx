import { useEffect, useState } from 'react'

import { errorText, agentStatus, importTheme, installTheme, openProviderSetup } from '../lib/ipc'
import {
  ACCENTS, AGENTS, PRESETS, findTheme, resetAppearance, type AgentId, type AgentStatus, type ColorTheme, type Preferences,
} from '../lib/settings'
import { AboutPane } from './AboutPane'
import type { WorkspaceInfo } from '../lib/events'
import { AgentMark } from './AgentMark'
import { CREW, crewRoles, systemPrompt } from '../lib/crew'
import { CrewPortrait } from './CrewPortrait'
import { SpeakSettings } from './SpeakSettings'
import { VoiceSettings } from './VoiceSettings'
import { GeneralSettings } from './GeneralSettings'
import { SchedulesSettings } from './SchedulesSettings'
import { Group, Toggle } from './SettingsRows'
import { ToolsSettings } from './ToolsSettings'
import { ConnectionsSettings } from './ConnectionsSettings'

const SECTIONS = [
  ['Recommended', '✧'], ['General', '◈'], ['Appearance', '◐'],
  ['Voice', '◉'], ['Agents', '◇'], ['Schedules', '◷'],
  ['Tools', '›_'], ['Connections', '◎'], ['About', 'ⓘ'],
] as const
export type Section = typeof SECTIONS[number][0]

type Props = {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
  workspace: WorkspaceInfo | null
  onClose: () => void
  initialSection: Section
  initialTab: string
}

const agentIds = Object.keys(AGENTS) as AgentId[]

export function Settings({ preferences: p, update, workspace, onClose, initialSection, initialTab }: Props) {
  const [section, setSection] = useState<Section>(initialSection)
  const [tab, setTab] = useState(initialTab)
  const [statuses, setStatuses] = useState<AgentStatus[]>([])
  const [checking, setChecking] = useState(false)
  const [selectedAgent, selectAgent] = useState<AgentId | null>(null)
  const [themeId, setThemeId] = useState('')
  const [themeBusy, setThemeBusy] = useState(false)
  const [status, setStatus] = useState('')
  const [checkInput, setCheckInput] = useState(() => JSON.stringify(p.checks[workspace?.root ?? ''] ?? []))
  const root = workspace?.root ?? ''
  // The custom accent is stored per scheme, and App paints the current scheme
  // onto the document, so read it back rather than threading the OS media query in.
  const lightScheme = document.documentElement.dataset.appearance === 'light'
  const customColor = lightScheme ? p.customAccentLight : p.customAccent
  const applyCustom = (color: string) => {
    const patch: Partial<Preferences> = {
      accent: 'Custom',
      recentAccents: [color, ...p.recentAccents.filter((entry) => entry !== color)].slice(0, 6),
    }
    if (lightScheme) patch.customAccentLight = color
    else patch.customAccent = color
    update(patch)
  }

  useEffect(() => {
    void agentStatus().then(setStatuses).catch((err) => setStatus(errorText(err)))
  }, [])

  const changeSection = (next: Section) => { setSection(next); setStatus('') }
  const checkAgents = async () => {
    setChecking(true)
    setStatus('')
    try {
      const results = await agentStatus(true)
      setStatuses(results)
      const updates = results.filter((entry) => entry.updateAvailable).length
      const compared = results.filter((entry) => entry.version && entry.latest).length
      setStatus(compared ? `${updates} updates available across ${compared} checked installations.`
        : 'Published versions could not be compared with the installed CLIs.')
    }
    catch (err) { setStatus(errorText(err)) }
    finally { setChecking(false) }
  }
  const applyTheme = async (action: () => Promise<ColorTheme[]>) => {
    setThemeBusy(true)
    setStatus('')
    try {
      const themes = await action()
      if (themes.length) {
        const merged = new Map([...p.themes, ...themes].map((theme) => [theme.id, theme]))
        update({ themes: [...merged.values()].slice(-48), themeId: themes[0].id })
      }
    } catch (err) { setStatus(errorText(err)) }
    finally { setThemeBusy(false) }
  }

  const modelRow = (id: AgentId, label: string) => (
    <label className="settings-row">
      <span><strong>{label}</strong><small>{AGENTS[id].models}. Empty uses the CLI default.</small></span>
      <input aria-label={label} value={p.models[id] ?? ''} placeholder="CLI default" maxLength={200}
        onChange={(event) => update({ models: { ...p.models, [id]: event.target.value } })} />
    </label>
  )

  const checks = (
    <Group title="Project checks">
      <form aria-label="Set project check command" onSubmit={(event) => {
        event.preventDefault()
        if (!root) return
        try {
          const args: unknown = JSON.parse(checkInput)
          if (!Array.isArray(args) || !args.every((arg) => typeof arg === 'string' && !arg.includes('\0'))
              || (args.length && !args[0].trim())) throw new Error()
          update({ checks: { ...p.checks, [root]: args } })
          setStatus(args.length ? 'Project check saved.' : 'Project check cleared.')
        } catch { setStatus('Enter a JSON array with an executable and its arguments.') }
      }}>
        <label className="settings-row">
          <span><strong>Command after successful turns</strong><small>{workspace?.name ?? 'Open a workspace to configure checks.'}</small></span>
          <input name="command" value={checkInput} disabled={!root} maxLength={4000} onChange={(event) => setCheckInput(event.target.value)}
            placeholder={'["cargo", "test"]'} />
        </label>
        <div className="settings-action"><button type="submit" disabled={!root}>Save command</button></div>
      </form>
    </Group>
  )

  return (
    <div className="settings-view">
      <aside className="settings-nav">
        <button type="button" className="settings-back" onClick={onClose} autoFocus>← <span>Back to workspace</span></button>
        <p className="settings-brand"><img src="/nova.webp" alt="" /> Nova Desktop</p>
        <nav aria-label="Settings sections">
          {SECTIONS.map(([item, glyph]) => (
            <button type="button" key={item} aria-current={section === item ? 'page' : undefined}
              onClick={() => changeSection(item)}>
              <span aria-hidden="true">{glyph}</span>{item}
            </button>
          ))}
        </nav>
      </aside>

      <main className="settings-stage" aria-label={section} key={section}>
        <div className="settings-content">
          <header className="settings-heading"><h1>{section}</h1></header>
          {section === 'Appearance' && <>
            <Group title="Display">
              <label className="settings-row">
                <span><strong>Theme</strong><small>Choose the base color scheme for Nova.</small></span>
                <select value={p.appearance} onChange={(event) => update({
                  appearance: event.target.value as Preferences['appearance'], themeId: '',
                })}>
                  <option value="system">System</option><option value="dark">Dark</option><option value="light">Light</option>
                </select>
              </label>
              <Toggle title="Wallpaper" detail="Show the night-sky illustration in your workspace."
                checked={p.wallpaper} onChange={(wallpaper) => update({ wallpaper })} />
              <Toggle title="Still sky" detail="Pause wallpaper motion."
                checked={p.still} onChange={(still) => update({ still })} />
            </Group>
            <Group title="Nova presets">
              <div className="settings-presets" aria-label="Color presets">
                {PRESETS.map((preset) => (
                  <button key={preset.id} type="button" aria-pressed={p.themeId === preset.id}
                    onClick={() => update({ themeId: preset.id })}>
                    <span className="settings-preset-chips" aria-hidden="true">
                      <i style={{ background: preset.colors['editor.background'] }} />
                      <i style={{ background: preset.colors.focusBorder }} />
                      <i style={{ background: preset.colors['editor.foreground'] }} />
                    </span>
                    <span>{preset.title}<small>{preset.mode}</small></span>
                  </button>
                ))}
              </div>
              <div className="settings-action">
                <button type="button" aria-pressed={p.themeId === ''} onClick={() => update({ themeId: '' })}>
                  Shipped look
                </button>
                <button type="button" onClick={() => update(resetAppearance(p))}>Reset appearance</button>
              </div>
              <p className="settings-help">
                A preset repaints the sidebar, chat ground, panels, dock and scrollbars together.
                Reset restores the shipped look and keeps the themes you installed.
              </p>
            </Group>
            <Group title="Accent color">
              <div className="settings-swatches" aria-label="Accent colors">
                {Object.entries(ACCENTS).map(([name, [color]]) => (
                  <button key={name} type="button" aria-pressed={p.accent === name}
                    onClick={() => update({ accent: name as keyof typeof ACCENTS })}>
                    <span className="settings-swatch" style={{ background: color }} />{name}
                  </button>
                ))}
                <label className="settings-custom">
                  <input type="color" aria-label="Custom accent" value={customColor}
                    onChange={(event) => applyCustom(event.target.value)} />
                  Custom
                </label>
              </div>
              {p.accentFromTheme && findTheme(p) && (
                <p className="settings-help">
                  This theme supplies the accent. Turn off “Use the theme's accent” to keep the one you picked.
                </p>
              )}
              {p.recentAccents.length > 0 && (
                <div className="settings-swatches" aria-label="Recent accent colors">
                  <span className="settings-hint">Recent</span>
                  {p.recentAccents.map((color) => (
                    <button key={color} type="button" aria-label={`Use ${color}`}
                      aria-pressed={p.accent === 'Custom' && color === customColor}
                      onClick={() => applyCustom(color)}>
                      <span className="settings-swatch" style={{ background: color }} />
                    </button>
                  ))}
                </div>
              )}
              <Toggle title="Use the theme's accent"
                detail="Colour themes carry their own accent; turn this off to keep the one you picked."
                checked={p.accentFromTheme} onChange={(accentFromTheme) => update({ accentFromTheme })} />
            </Group>
            <Group title="VS Code themes">
              <div className="settings-themes">
                {p.themes.map((theme) => (
                  <div className="settings-theme" key={theme.id}>
                    <button type="button" aria-pressed={p.themeId === theme.id} onClick={() => update({ themeId: theme.id })}>
                      <span className="settings-theme-sample" style={{ background: theme.colors['editor.background'] ?? 'var(--accent)' }} />
                      <span>{theme.title}<small>{theme.mode}</small></span>
                    </button>
                    {theme.id && <button type="button" aria-label={`Remove ${theme.title}`} onClick={() => update({
                      themes: p.themes.filter(({ id }) => id !== theme.id), themeId: p.themeId === theme.id ? '' : p.themeId,
                    })}>Remove</button>}
                  </div>
                ))}
              </div>
              <form className="settings-import" aria-label="Install color theme" onSubmit={(event) => {
                event.preventDefault()
                if (themeId.trim() && !themeBusy) void applyTheme(() => installTheme(themeId.trim()))
              }}>
                <button type="button" disabled={themeBusy} onClick={() => void applyTheme(importTheme)}>Import .vsix</button>
                <input name="extension" aria-label="Open VSX extension ID" placeholder="publisher.extension" maxLength={160} required
                  value={themeId} onChange={(event) => setThemeId(event.target.value)} />
                <button type="submit" disabled={themeBusy || !themeId.trim()}>{themeBusy ? 'Loading…' : 'Install'}</button>
              </form>
              <p className="settings-help">Install from Open VSX or import a theme archive. Nova reads its workbench colors.</p>
            </Group>
          </>}
          {section === 'Agents' && <>
            <nav className="settings-tabs" aria-label="Agent settings">
              {['Harnesses', 'Providers', 'Agent teams', 'Model routing'].map((item) => (
                <button key={item} type="button" aria-pressed={tab === item} onClick={() => { setTab(item); setStatus('') }}>{item}</button>
              ))}
            </nav>
            {tab === 'Harnesses' && <>
              <Group title="Defaults">
                <label className="settings-row">
                  <span><strong>Default agent</strong><small>Used when you start a new task.</small></span>
                  <select value={p.defaultAgent} onChange={(event) => update({ defaultAgent: event.target.value as AgentId })}>
                    {agentIds.map((id) => <option key={id} value={id}>{AGENTS[id].name}</option>)}
                  </select>
                </label>
                {modelRow(p.defaultAgent, 'Default model')}
                <div className="settings-row">
                  <span><strong>Agent updates</strong><small>Compare local versions with published releases.</small></span>
                  <button type="button" disabled={checking} onClick={() => void checkAgents()}>
                    {checking ? 'Checking…' : 'Check updates'}
                  </button>
                </div>
                <Toggle title="Project checks after turns" detail={root ? 'Runs the saved command after a successful turn.' : 'Open a workspace to configure checks.'}
                  checked={!!root && Boolean(p.checks[root]?.length)}
                  onChange={(enabled) => {
                    if (enabled) changeSection('Tools')
                    else if (root) {
                      update({ checks: { ...p.checks, [root]: [] } })
                      setCheckInput('[]')
                    }
                  }} />
              </Group>
              <Group title="Installed agents">
                {agentIds.map((id) => {
                  const found = statuses.find((entry) => entry.id === id)
                  return (
                    <div key={id}>
                      <button className="settings-row agent-status" type="button" aria-expanded={selectedAgent === id}
                        onClick={() => selectAgent(selectedAgent === id ? null : id)}>
                        <span className="agent-title"><AgentMark id={id} chip />
                          <strong>{AGENTS[id].name}</strong></span>
                        <span className="agent-indicator" data-installed={found?.installed}>
                          {found?.installed ? found.version ?? 'Installed' : found ? 'Not found' : 'Checking…'}
                          {found?.updateAvailable && <small>Update {found.latest}</small>} <span aria-hidden="true">›</span>
                        </span>
                      </button>
                      {selectedAgent === id && <div className="agent-detail">
                        <p>{found?.executable ?? 'Install this agent on PATH to use it in a workspace.'}</p>
                        {found?.latest && <p>Latest published version <code>{found.latest}</code></p>}
                        <button type="button" onClick={() => update({ defaultAgent: id })}>Use as default</button>
                      </div>}
                    </div>
                  )
                })}
              </Group>
            </>}
            {tab === 'Providers' && <Group title="Provider setup">
              <p className="settings-help">Desktop uses each agent’s existing login and provider settings.</p>
              {agentIds.map((id) => <div className="settings-row" key={id}>
                <span className="agent-title"><AgentMark id={id} />
                  <span><strong>{AGENTS[id].name}</strong><small>{AGENTS[id].setup}</small></span></span>
                <button type="button" onClick={async () => {
                  try { await openProviderSetup(id); setStatus('Complete setup in the terminal window.') }
                  catch (err) { setStatus(errorText(err)) }
                }}>Open setup</button>
              </div>)}
            </Group>}
             {tab === 'Agent teams' && <>
              <Group title="Team pipeline">
                <p className="settings-help">@team runs Nova → the enabled workers → a final report by Nova. These settings shape that pipeline.</p>
                <Toggle title="Require plan approval" detail="Pause the run after Nova's plan until you approve, edit or skip it."
                  checked={p.team.planApproval}
                  onChange={(enabled) => update({ team: { ...p.team, planApproval: enabled } })} />
                <label className="settings-row">
                  <span><strong>Ralph iterations</strong><small>How often a genuinely failing check is retried before the run moves on.</small></span>
                  <select aria-label="Ralph iterations" value={p.team.ralphIterations}
                    onChange={(event) => update({ team: { ...p.team, ralphIterations: Number(event.target.value) } })}>
                    {[1, 2, 3, 4, 5].map((count) => <option key={count} value={count}>{count}</option>)}
                  </select>
                </label>
                <label className="settings-row">
                  <span><strong>Stage watchdog</strong><small>Fail a stage that produces no output for this long, and let the team continue.</small></span>
                  <select aria-label="Stage watchdog" value={p.team.stageTimeoutMin}
                    onChange={(event) => update({ team: { ...p.team, stageTimeoutMin: Number(event.target.value) } })}>
                    {[2, 5, 10, 15].map((minutes) => <option key={minutes} value={minutes}>{minutes} min</option>)}
                  </select>
                </label>
                <div className="settings-row">
                  <span><strong>Pipeline workers</strong><small>Which agents run between Nova's plan and the final report.</small></span>
                  <span className="crew-pipeline-roles">
                    {crewRoles.filter((role) => role !== 'nova').map((role) => {
                      const enabled = p.team.roles[role] !== false
                      return <label key={role} className="crew-role-toggle">
                        <input type="checkbox" checked={enabled} onChange={(event) => update({
                          team: { ...p.team, roles: { ...p.team.roles, [role]: event.target.checked } },
                        })} />
                        <CrewPortrait role={role} />
                        {CREW[role].name}
                      </label>
                    })}
                  </span>
                </div>
              </Group>
              <Group title="Agent personalities">
              <p className="settings-help">Address a message with @nova, @serena, @ralph, @nora, or @team. Each personality uses the harness, model, and system prompt set here.</p>
              {crewRoles.map((role) => {
                const profile = p.crew[role] ?? { agent: 'nova' }
                const actor = CREW[role]
                return <section className="crew-profile" key={role}>
                  <header className="reply-speaker">
                    <CrewPortrait role={role} />
                    <div><strong>{actor.name}</strong><small>{actor.title}</small></div>
                  </header>
                  {role === 'nora' && <p>Browser research uses the browser and search tools configured in the selected harness.</p>}
                  <div className="crew-profile-fields">
                    <label>Harness<select aria-label={actor.name + ' harness'} value={profile.agent}
                      onChange={(event) => update({ crew: { ...p.crew, [role]: {
                        ...profile, agent: event.target.value as AgentId, model: undefined,
                      } } })}>
                      {agentIds.map((id) => <option key={id} value={id}>{AGENTS[id].name}</option>)}
                    </select></label>
                    <label>Model<input aria-label={actor.name + ' model'} value={profile.model ?? ''}
                      placeholder={AGENTS[profile.agent].models + ' or harness default'} maxLength={200}
                      onChange={(event) => update({ crew: { ...p.crew, [role]: { ...profile, model: event.target.value } } })} /></label>
                  </div>
                  <label>System prompt<textarea aria-label={actor.name + ' system prompt'} rows={3}
                    value={profile.systemPrompt ?? systemPrompt(role)} maxLength={2000}
                    placeholder="Use the default personality prompt"
                    onChange={(event) => update({ crew: { ...p.crew, [role]: { ...profile, systemPrompt: event.target.value } } })} /></label>
                </section>
              })}
              </Group>
            </>}
            {tab === 'Model routing' && <>
              <Group title="Workspace routing">
                <label className="settings-row">
                  <span><strong>Default for this workspace</strong><small>{workspace?.name ?? 'Open a workspace to set a route.'}</small></span>
                  <select disabled={!root} value={p.routes[root] ?? ''} onChange={(event) => {
                    const routes = { ...p.routes }
                    if (event.target.value) routes[root] = event.target.value as AgentId
                    else delete routes[root]
                    update({ routes })
                  }}>
                    <option value="">App default</option>
                    {agentIds.map((id) => <option key={id} value={id}>{AGENTS[id].name}</option>)}
                  </select>
                </label>
              </Group>
              <Group title="Model overrides">{agentIds.map((id) => <div key={id}>{modelRow(id, AGENTS[id].name)}</div>)}</Group>
            </>}
          </>}
          {section === 'General' && <GeneralSettings preferences={p} update={update} workspace={workspace} />}
          {section === 'Voice' && <>
            {/* Reading replies aloud stays first: it is the one voice feature that
                needs no download, and it is already switched on for some people.
                The toggle, the destination and the preview all live in
                SpeakSettings now, so there is one place that knows them. */}
            <SpeakSettings preferences={p} update={update} />
            <VoiceSettings preferences={p} update={update} />
          </>}
          {section === 'Tools' && (
            /* Two tabs, two questions: Add-ons is the summary — the switch, the
               endpoint, and a Manage button that opens the counted sheet — and
               Skills is the folder listing, now with a switch per row. The
               Markdownify form moved to General: it converts a paste, it is not
               a tool surface. The Project checks form goes in as `extras` so it
               lands inside Add-ons instead of floating above the tab bar. */
            <ToolsSettings extras={checks} preferences={p} update={update}
              onChangeSection={changeSection} />
          )}
          {section === 'Connections' && <ConnectionsSettings key={root} workspaceReady={Boolean(root)} />}
          {section === 'Schedules' && <SchedulesSettings preferences={p} update={update} workspace={workspace} />}
          {section === 'Recommended' && <Group title="Set up your workspace">
            <div className="settings-row"><span><strong>Choose your default agent</strong><small>Configure the harness and model used for new tasks.</small></span>
              <button type="button" onClick={() => changeSection('Agents')}>Open Agents</button></div>
            <div className="settings-row"><span><strong>Check your tool connections</strong><small>Review the MCP servers configured for Nova.</small></span>
              <button type="button" onClick={() => changeSection('Connections')}>Open Connections</button></div>
          </Group>}
          {section === 'About' && <AboutPane preferences={p} update={update} />}
          <p className="settings-status" role="status">{status}</p>
        </div>
      </main>
    </div>
  )
}
