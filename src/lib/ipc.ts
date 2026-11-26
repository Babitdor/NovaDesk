import { Channel, invoke } from '@tauri-apps/api/core'

import type { AgentEvent, DirEntry, InterruptReply, WorkspaceInfo } from './events'
import type { CrewRole } from './crew'
import type { AgentStatus, ColorTheme } from './settings'

export const pickWorkspace = () => invoke<WorkspaceInfo | null>('pick_workspace')
export const setWorkspace = (path: string) => invoke<WorkspaceInfo>('set_workspace', { path })

export const listDir = (path: string) => invoke<DirEntry[]>('list_dir', { path })

export const readFile = (path: string) => invoke<string>('read_file', { path })

/// The handler must be attached before the invoke, or deltas that arrive while
/// the command starts are dropped.
function channelFor(onEvent: (event: AgentEvent) => void) {
  const channel = new Channel<AgentEvent>()
  channel.onmessage = onEvent
  return channel
}

export const startTurn = (
  request: { turnId: string; chatId: string; prompt: string; backend: string; model?: string; effort?: string; session?: string; check?: string[]; trustWorkspace: boolean; personality?: { role: CrewRole; systemPrompt: string } },
  onEvent: (event: AgentEvent) => void,
) => invoke<void>('start_turn', { ...request, onEvent: channelFor(onEvent) })

export const cancelTurn = (turnId: string) => invoke<boolean>('cancel_turn', { turnId })

/// Answers a backend that is blocked on a decision; the turn stays open until
/// this resolves.
export const respondApproval = (turnId: string, interruptId: string, reply: InterruptReply) =>
  invoke<void>('respond_approval', { turnId, interruptId, reply })

export const agentStatus = (versions = false) => invoke<AgentStatus[]>('agent_status', { versions })

/// Spawns Nova's pipe session for the open workspace in the background; the
/// first turn of a task then streams immediately instead of paying the boot.
export const prewarmNova = () => invoke<void>('prewarm_nova')
export const importTheme = () => invoke<ColorTheme[]>('import_theme')
export const installTheme = (id: string) => invoke<ColorTheme[]>('install_theme', { id })
export const novaMcpList = () => invoke<string>('nova_mcp_list')
export const addMcpServer = (name: string, endpoint: string) => invoke<void>('add_mcp_server', { name, endpoint })
export const openProviderSetup = (id: string) => invoke<void>('open_provider_setup', { id })
export const openNovaTerminal = () => invoke<void>('open_nova_terminal')
export const markdownify = (html: string) => invoke<string>('markdownify', { html })
export const openLink = (url: string) => invoke<void>('open_link', { url })

// --- about ----------------------------------------------------------
// Mirrors `AppReport` and `UpdateCheck` in src-tauri/src/commands/app.rs.
export type AppReport = {
  version: string
  identifier: string
  os: string
  arch: string
  /// Working set of the native process, in bytes; null when the query failed.
  rssBytes: number | null
  uptimeSecs: number
  agents: number
  terminals: number
  dataDir: string | null
}

export type UpdateCheck = {
  current: string
  latest: string
  url: string | null
  publishedAt: string | null
  notes: string | null
  available: boolean
}

export const appReport = () => invoke<AppReport>('app_report')
export const checkAppUpdate = () => invoke<UpdateCheck>('check_app_update')

export const runCommand = (
  commandId: string,
  program: string,
  args: string[],
  onEvent: (event: AgentEvent) => void,
) => invoke<void>('run_command', { commandId, program, args, onEvent: channelFor(onEvent) })

export const killProcess = (commandId: string) => invoke<boolean>('kill_process', { commandId })

export type TerminalEvent =
  | { type: 'screen'; lines: string[]; row: number; column: number }
  | { type: 'error'; message: string }
  | { type: 'exit' }

export const terminalShells = {
  powershell: 'PowerShell',
  cmd: 'Command Prompt',
  gitbash: 'Git Bash',
} as const

export type TerminalShell = keyof typeof terminalShells

export const startTerminal = (id: string, shell: TerminalShell, rows: number, cols: number, onEvent: (event: TerminalEvent) => void) => {
  const channel = new Channel<TerminalEvent>(onEvent)
  return invoke<void>('start_terminal', { id, shell, rows, cols, onEvent: channel })
}

export const closeTerminal = (id: string) => invoke<void>('close_terminal', { id })
export const terminalInput = (id: string, text: string) => invoke<void>('terminal_input', { id, text })
export const terminalResize = (id: string, rows: number, cols: number) => invoke<void>('terminal_resize', { id, rows, cols })

// --- embedded browser ------------------------------------------------
// The browser tab is a native child webview: it is created, moved and hidden
// from here, and it paints above the page, so every layout change has to reach
// it or the panel looks broken.

export const browserOpen = (url: string, x: number, y: number, w: number, h: number) =>
  invoke<void>('browser_open', { url, x, y, width: w, height: h })

export const browserBounds = (x: number, y: number, w: number, h: number) =>
  invoke<void>('browser_bounds', { x, y, width: w, height: h })

export const browserVisibility = (visible: boolean) =>
  invoke<void>('browser_visibility', { visible })

export const browserReload = () => invoke<void>('browser_reload')

export const browserHistory = (direction: 'back' | 'forward') =>
  invoke<void>('browser_history', { direction })

export const browserClose = () => invoke<void>('browser_close')

// --- launcher, widget and crash recovery ------------------------------
// The two auxiliary windows are created and placed by Rust; the page only asks
// for a state. Mirrors commands::launcher and commands::recovery.

/// Registers (or clears) the global chord and remembers it in the app data dir.
/// Rejects with the platform's own message when the chord is already taken.
export const setQuickLauncher = (enabled: boolean, accelerator: string) =>
  invoke<void>('set_quick_launcher', { enabled, accelerator })

export const openLauncher = () => invoke<void>('open_launcher')

export const setWidget = (enabled: boolean, pinned: boolean) =>
  invoke<void>('set_widget', { enabled, pinned })

/// Mirrors `CrashReport` in src-tauri/src/recovery.rs.
export type CrashReport = {
  path: string
  at: number
  source: string
  message: string
  location: string
  backtrace: string
}

export const setCrashPolicy = (restartAfterCrash: boolean, crashReports: boolean) =>
  invoke<void>('set_crash_policy', { restartAfterCrash, crashReports })

export const crashReports = () => invoke<CrashReport[]>('crash_reports')
export const clearCrashReports = () => invoke<void>('clear_crash_reports')
export const reportCrash = (message: string, location: string, source = 'webview') =>
  invoke<void>('report_crash', { message, location, source })
export const browserLocalServers = () => invoke<string[]>('browser_local_servers')

/// Commands reject with `{ kind, message }` (see error.rs), not an `Error`, so
/// `String(err)` would print "[object Object]".
export function errorText(err: unknown): string {
  if (typeof err === 'object' && err !== null && 'message' in err) {
    return String((err as { message: unknown }).message)
  }
  return String(err)
}
