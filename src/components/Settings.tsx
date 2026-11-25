import { useEffect, useState, type ReactNode } from 'react'
import { getVersion } from '@tauri-apps/api/app'

import { addMcpServer, errorText, agentStatus, importTheme, installTheme, markdownify, novaMcpList, openProviderSetup } from '../lib/ipc'
import {
  ACCENTS, AGENTS, type AgentId, type AgentStatus, type ColorTheme, type Preferences,
} from '../lib/settings'
import type { WorkspaceInfo } from '../lib/events'
import { AgentMark } from './AgentMark'
import { Markdown } from './Markdown'
import { CREW, crewRoles, systemPrompt } from '../lib/crew'
import { CrewPortrait } from './CrewPortrait'

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

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="settings-group">
      {title && <h2>{title}</h2>}
      {children}
    </section>
  )
}

function Toggle({ title, detail, checked, onChange }: {
  title: string; detail?: string; checked: boolean; onChange: (value: boolean) => void
}) {
  return (
    <label className="settings-row">
      <span><strong>{title}</strong>{detail && <small>{detail}</small>}</span>
      <input {...{ switch: '' }} className="settings-switch" type="checkbox" role="switch"
        checked={checked} onChange={(event) => onChange(event.target.checked)} />
    </label>
  )
}

export function Settings({ preferences: p, update, workspace, onClose, initialSection, initialTab }: Props) {
  const [section, setSection] = useState<Section>(initialSection)
  const [tab, setTab] = useState(initialTab)
  const [statuses, setStatuses] = useState<AgentStatus[]>([])
  const [checking, setChecking] = useState(false)
  const [selectedAgent, selectAgent] = useState<AgentId | null>(null)
  const [themeId, setThemeId] = useState('')
  const [themeBusy, setThemeBusy] = useState(false)
  const [status, setStatus] = useState('')
  const [version, setVersion] = useState('')
  const [servers, setServers] = useState<string | null>(null)
  const [mcpBusy, setMcpBusy] = useState(false)
  const [checkInput, setCheckInput] = useState(() => JSON.stringify(p.checks[workspace?.root ?? ''] ?? []))
  const [html, setHtml] = useState('')
  const [converted, setConverted] = useState('')
  const [converting, setConverting] = useState(false)
  const root = workspace?.root ?? ''

  useEffect(() => {
    void agentStatus().then(setStatuses).catch((err) => setStatus(errorText(err)))
    void getVersion().then(setVersion).catch(() => {})
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
            <Group title="Accent color">
              <div className="settings-swatches" aria-label="Accent colors">
                {Object.entries(ACCENTS).map(([name, [color]]) => (
                  <button key={name} type="button" aria-pressed={p.accent === name}
                    onClick={() => update({ accent: name as keyof typeof ACCENTS })}>
                    <span className="settings-swatch" style={{ background: color }} />{name}
                  </button>
                ))}
                <label className="settings-custom">
                  <input type="color" aria-label="Custom accent" value={p.customAccent}
                    onChange={(event) => update({ accent: 'Custom', customAccent: event.target.value })} />
                  Custom
                </label>
              </div>
            </Group>
            <Group title="VS Code themes">
              <div className="settings-themes">
                {([{ id: '', title: 'Nova', mode: 'Built in', colors: {} as ColorTheme['colors'] }, ...p.themes]).map((theme) => (
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
            {tab === 'Agent teams' && <Group title="Agent personalities">
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
            </Group>}
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
          {section === 'General' && <Group title="Workspace behavior">
            <Toggle title="Reopen last folder" checked={p.restoreFolder} onChange={(restoreFolder) => update({ restoreFolder })} />
            <Toggle title="Render Markdown in chat" checked={p.markdown} onChange={(markdown) => update({ markdown })} />
            {root && <Toggle title="Allow Nova to trust this folder on startup"
              detail="Nova saves the folder approval in its CLI configuration. Revoke it with nova paths revoke."
              checked={p.trustedWorkspaces.includes(root)} onChange={(enabled) => update({
                trustedWorkspaces: enabled ? [...p.trustedWorkspaces, root]
                  : p.trustedWorkspaces.filter((path) => path !== root),
              })} />}
          </Group>}
          {section === 'Voice' && <Group title="Read responses aloud">
            <Toggle title="Speak finished replies" detail="Uses your operating system’s available voices."
              checked={p.speak} onChange={(speak) => update({ speak })} />
            <div className="settings-action">
              <button type="button" disabled={!('speechSynthesis' in window)}
                onClick={() => window.speechSynthesis.speak(new SpeechSynthesisUtterance('Nova Desktop is ready.'))}>Preview voice</button>
              <button type="button" disabled={!('speechSynthesis' in window)} onClick={() => window.speechSynthesis.cancel()}>Stop speaking</button>
            </div>
          </Group>}
          {section === 'Tools' && <>
            {checks}
            <Group title="Markdownify">
              <form className="settings-convert" aria-label="Convert HTML to Markdown" onSubmit={async (event) => {
                event.preventDefault()
                setConverting(true)
                setStatus('')
                try { setConverted(await markdownify(html)) }
                catch (err) { setStatus(errorText(err)) }
                finally { setConverting(false) }
              }}>
                <label>HTML<textarea name="html" value={html} onChange={(event) => setHtml(event.target.value)} rows={5} required
                  placeholder="<h1>Hello</h1>" maxLength={512 * 1024} /></label>
                <div className="settings-action"><button type="submit" disabled={converting}>
                  {converting ? 'Converting…' : 'Convert to Markdown'}
                </button></div>
                {converted && <>
                  <label>Markdown<textarea value={converted} readOnly rows={5} /></label>
                  <Markdown text={converted} />
                </>}
              </form>
            </Group>
          </>}
          {section === 'Connections' && <Group title="Nova MCP servers">
            <p className="settings-help">Reads configured servers from the current workspace using Nova’s CLI.</p>
            <div className="settings-action"><button type="button" disabled={!root || mcpBusy} onClick={async () => {
              setMcpBusy(true)
              setStatus('')
              try { setServers(await novaMcpList()) }
              catch (err) { setStatus(errorText(err)) }
              finally { setMcpBusy(false) }
            }}>{mcpBusy ? 'Loading…' : 'List servers'}</button></div>
            {servers !== null && <pre className="settings-output">{servers || 'No servers reported.'}</pre>}
            <form className="settings-convert" aria-label="Add MCP server" onSubmit={async (event) => {
              event.preventDefault()
              const form = event.currentTarget
              const fields = new FormData(form)
              setMcpBusy(true)
              setStatus('')
              try {
                await addMcpServer(String(fields.get('name')).trim(), String(fields.get('endpoint')).trim())
                form.reset()
                setStatus('Server saved. New Nova sessions load the updated configuration.')
                setServers(await novaMcpList())
              } catch (err) { setStatus(errorText(err)) }
              finally { setMcpBusy(false) }
            }}>
              <label>Server name<input name="name" required maxLength={64} disabled={!root || mcpBusy} /></label>
              <label>HTTP endpoint<input name="endpoint" type="url" required placeholder="https://server.example/mcp"
                maxLength={2048} disabled={!root || mcpBusy} /></label>
              <div className="settings-action"><button type="submit" disabled={!root || mcpBusy}>Add server</button></div>
            </form>
          </Group>}
          {section === 'Schedules' && <Group title="Scheduled prompts">
            <p className="settings-help">Runs when Nova Desktop is open, the same folder is active, and no turn is running.</p>
              <form className="settings-convert" aria-label="Schedule a prompt" onSubmit={(event) => {
              event.preventDefault()
              if (!root) return
              const form = new FormData(event.currentTarget)
              const prompt = String(form.get('prompt') ?? '').trim()
              const at = new Date(String(form.get('at') ?? '')).getTime()
              if (!prompt || !Number.isFinite(at) || at <= Date.now()) {
                setStatus('Enter a prompt and a future date and time.')
                return
              }
              const agent = String(form.get('agent')) as AgentId
              update({ schedules: [...p.schedules, { id: crypto.randomUUID(), prompt, workspace: root,
                agent, at, daily: form.has('daily') }] })
              event.currentTarget.reset()
              setStatus('Prompt scheduled.')
            }}>
              <label>Prompt<textarea name="prompt" required rows={3} maxLength={12000} disabled={!root} /></label>
              <label>Run at<input type="datetime-local" name="at" required disabled={!root} /></label>
              <label>Agent<select name="agent" defaultValue={p.defaultAgent} disabled={!root}>
                {agentIds.map((id) => <option key={id} value={id}>{AGENTS[id].name}</option>)}
              </select></label>
              <label className="settings-inline"><input type="checkbox" name="daily" disabled={!root} /> Repeat daily</label>
              <div className="settings-action"><button type="submit" disabled={!root}>Schedule prompt</button></div>
            </form>
            {p.schedules.map((item) => <div key={item.id} className="settings-row">
              <span><strong>{item.prompt.slice(0, 80)}</strong><small>
                {new Date(item.at).toLocaleString()} · {AGENTS[item.agent].name}{item.daily ? ' · Daily' : ''}
              </small></span>
              <button type="button" onClick={() => update({ schedules: p.schedules.filter(({ id }) => id !== item.id) })}>Remove</button>
            </div>)}
          </Group>}
          {section === 'Recommended' && <Group title="Set up your workspace">
            <div className="settings-row"><span><strong>Choose your default agent</strong><small>Configure the harness and model used for new tasks.</small></span>
              <button type="button" onClick={() => changeSection('Agents')}>Open Agents</button></div>
            <div className="settings-row"><span><strong>Check your tool connections</strong><small>Review the MCP servers configured for Nova.</small></span>
              <button type="button" onClick={() => changeSection('Connections')}>Open Connections</button></div>
          </Group>}
          {section === 'About' && <Group title="Nova Desktop">
            <div className="settings-row"><strong>Version</strong><span>{version || 'Development build'}</span></div>
            <div className="settings-row"><span><strong>Workspace</strong><small>{workspace?.root ?? 'No folder open'}</small></span></div>
          </Group>}
          <p className="settings-status" role="status">{status}</p>
        </div>
      </main>
    </div>
  )
}
