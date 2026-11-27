import { invoke } from '@tauri-apps/api/core'

/// Mirrors the registry in src-tauri/src/tools/mod.rs. The pane renders what
/// this returns and nothing else, so a tool that is not in the table cannot be
/// shown and one in the table cannot be hidden.
export type Injection = 'flags' | 'env' | 'projectFile' | 'configFile' | 'none'
export type ToolGroup = 'tasks' | 'queue' | 'browser' | 'files' | 'discovery'

export type ToolRow = {
  id: string
  title: string
  description: string
  group: ToolGroup
  handler: 'native' | 'frontend'
  requiresWindow: boolean
  schema: unknown
}

export type HarnessRow = {
  id: string
  name: string
  injection: Injection
  /// True when this launch hands the harness the server — per turn, or via a
  /// managed workspace entry for nova.
  callable: boolean
  note: string | null
}

export type Catalog = {
  enabled: boolean
  endpoint: string | null
  harnesses: HarnessRow[]
  tools: ToolRow[]
  listedTools?: string[]
}

export const toolsCatalog = () => invoke<Catalog>('tools_catalog')
export const setToolsEnabled = (enabled: boolean) => invoke<void>('set_tools_enabled', { enabled })

/// Skill directories the user switched off. Rust keeps the list and leaves
/// those skills out of what list_skills answers, so the switch is not a label.
export const setDisabledSkills = (ids: string[]) => invoke<void>('set_disabled_skills', { ids })

/// One MCP server as its own harness reports it. `detail` is that CLI's words,
/// including its connected/failed marker: Nova reads the list, it does not
/// interpret it.
export type McpServer = {
  harnessId: string
  harness: string
  name: string
  detail: string
}

/// The discovery result. `timedOut` and `unsupported` name the harnesses that
/// could not answer, kept apart from an empty list.
export type McpDiscovery = {
  servers: McpServer[]
  timedOut: string[]
  unsupported: string[]
}

export const mcpServers = () => invoke<McpDiscovery>('mcp_servers')
export const registerCommandCodeTools = () => invoke<void>('register_commandcode_tools')

/// Answers the one-shot Rust is waiting on. The shape is the contract:
/// { ok: true, value } or { ok: false, error }.
export type ToolReply = { ok: true; value: unknown } | { ok: false; error: string }
export const toolReply = (id: string, reply: ToolReply) =>
  invoke<boolean>('tool_reply', { id, result: reply })

export const INJECTION_LABELS: Record<Injection, string> = {
  flags: 'Injected per turn (argv)',
  env: 'Injected per turn (env)',
  projectFile: 'Managed entry in the workspace config',
  configFile: 'Config file only',
  none: 'Not supported',
}

export const GROUP_LABELS: Record<ToolGroup, string> = {
  tasks: 'Tasks',
  queue: 'Queue and steering',
  browser: 'Browser',
  files: 'Files and search',
  discovery: 'Discovery',
}
