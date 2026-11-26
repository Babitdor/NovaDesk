import { invoke } from '@tauri-apps/api/core'

/// Mirrors the registry in src-tauri/src/tools/mod.rs. The pane renders what
/// this returns and nothing else, so a tool that is not in the table cannot be
/// shown and one in the table cannot be hidden.
export type Injection = 'flags' | 'env' | 'configFile' | 'none'
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
  /// True when this launch can hand the harness the server for one turn.
  callable: boolean
  note: string | null
}

export type Catalog = {
  enabled: boolean
  endpoint: string | null
  harnesses: HarnessRow[]
  tools: ToolRow[]
}

export const toolsCatalog = () => invoke<Catalog>('tools_catalog')
export const setToolsEnabled = (enabled: boolean) => invoke<void>('set_tools_enabled', { enabled })

/// Answers the one-shot Rust is waiting on. The shape is the contract:
/// { ok: true, value } or { ok: false, error }.
export type ToolReply = { ok: true; value: unknown } | { ok: false; error: string }
export const toolReply = (id: string, reply: ToolReply) =>
  invoke<boolean>('tool_reply', { id, result: reply })

export const INJECTION_LABELS: Record<Injection, string> = {
  flags: 'Injected per turn (argv)',
  env: 'Injected per turn (env)',
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
