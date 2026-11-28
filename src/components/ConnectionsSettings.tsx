import { useEffect, useId, useRef, useState, type FormEvent } from 'react'

import { addMcpServer, errorText } from '../lib/ipc'
import { AGENTS, type AgentId } from '../lib/settings'
import { mcpServers, registerCommandCodeTools, toolsCatalog, type McpDiscovery } from '../lib/tools'
import { ActionButton, Group } from './SettingsRows'

export function ConnectionsSettings({ workspaceReady }: { workspaceReady: boolean }) {
  const [selected, select] = useState<AgentId>('nova')
  const [inventory, setInventory] = useState<McpDiscovery | null>(null)
  const [listed, setListed] = useState<string[]>([])
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState('')
  const request = useRef(0)
  const id = useId()

  async function refresh() {
    const current = ++request.current
    setLoading(true)
    const [servers, catalog] = await Promise.allSettled([mcpServers(), toolsCatalog()])
    if (request.current !== current) return
    if (servers.status === 'fulfilled') setInventory(servers.value)
    else setStatus(errorText(servers.reason))
    if (catalog.status === 'fulfilled') setListed(catalog.value.listedTools ?? [])
    setLoading(false)
  }

  useEffect(() => {
    const pending = setTimeout(() => void refresh(), 0)
    return () => {
      clearTimeout(pending)
      request.current += 1
    }
  }, [])

  async function save(action: () => Promise<void>, message: string) {
    setSaving(true)
    setStatus('')
    try {
      await action()
      setStatus(message)
      await refresh()
    } catch (err) { setStatus(errorText(err)) }
    finally { setSaving(false) }
  }

  function add(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!workspaceReady || saving) return
    const form = event.currentTarget
    const fields = new FormData(form)
    void save(async () => {
      await addMcpServer(String(fields.get('name')).trim(), String(fields.get('endpoint')).trim())
      form.reset()
    }, 'Server saved. New Nova sessions load the updated configuration.')
  }

  const harness = AGENTS[selected].name
  const servers = inventory?.servers.filter((server) => server.harnessId === selected) ?? []
  const registered = servers.some((server) => server.name === 'nova-desktop')

  return <>
    <Group title="MCP servers">
      <div className="settings-row">
        <label htmlFor={id + '-harness'}>Harness</label>
        <select id={id + '-harness'} value={selected} disabled={saving} onChange={(event) => {
          select(event.target.value as AgentId)
          setStatus('')
        }}>
          {Object.entries(AGENTS).map(([id, agent]) => <option key={id} value={id}>{agent.name}</option>)}
        </select>
        <ActionButton icon="refresh" type="button" disabled={loading || saving} onClick={() => { setStatus(''); void refresh() }}>
          {loading ? 'Checking…' : 'Refresh'}
        </ActionButton>
      </div>
      {listed.includes(selected) && <p className="settings-help">{harness} listed Nova Desktop tools this launch.</p>}
      {servers.map((server, index) => <div className="settings-row" key={server.name + ':' + index}>
        <span><strong>{server.name}</strong><small>{server.detail}</small></span>
      </div>)}
      <p className="settings-help" role="status">
        {loading ? `Checking ${harness}…`
          : inventory?.timedOut.includes(harness) ? `${harness} timed out while listing servers.`
            : inventory?.unsupported.includes(harness) ? `${harness} could not list its configured servers.`
              : inventory && !servers.length ? `${harness} reported no configured servers.` : ''}
      </p>
    </Group>
    {selected === 'nova' && <Group title="Add a Nova MCP server">
      <p className="settings-help">{workspaceReady
        ? 'Add an HTTP or HTTPS server to your Nova CLI profile.'
        : 'Open a workspace to add a server.'}</p>
      <form className="settings-convert" aria-label="Add a Nova MCP server" onSubmit={add}>
        <label htmlFor={id + '-name'}>Server name
          <input id={id + '-name'} name="name" required maxLength={64} pattern={'[A-Za-z0-9][A-Za-z0-9_\\-]*'}
            title="Start with a letter or number. Use letters, numbers, hyphens, or underscores."
            disabled={!workspaceReady || saving} />
        </label>
        <label htmlFor={id + '-endpoint'}>HTTP endpoint
          <input id={id + '-endpoint'} name="endpoint" type="url" pattern="https?://.+" required
            placeholder="https://server.example/mcp" maxLength={2048} disabled={!workspaceReady || saving} />
        </label>
        <div className="settings-action"><ActionButton icon="plus" type="submit" disabled={!workspaceReady || saving}>
          {saving ? 'Saving…' : 'Add server'}
        </ActionButton></div>
      </form>
    </Group>}
    {selected === 'commandcode' && <Group title="Nova Desktop tools">
      <p className="settings-help">{workspaceReady
        ? 'Register the shim in CommandCode’s private configuration for this project.'
        : 'Open a workspace to register the shim.'}</p>
      <div className="settings-action"><ActionButton icon={registered ? 'check' : 'plus'} type="button"
        disabled={!workspaceReady || loading || saving || registered} onClick={() => {
          void save(registerCommandCodeTools, 'Registered for this project. Start a CommandCode turn to check the connection.')
        }}>{registered ? 'Registered' : saving ? 'Registering…' : 'Register for this project'}</ActionButton></div>
    </Group>}
    {selected === 'hermes' && <Group title="Hermes configuration">
      <p className="settings-help">Manage servers in your Hermes profile with <code>hermes mcp</code>.</p>
    </Group>}
    <p className="settings-status" role="status">{status}</p>
  </>
}
