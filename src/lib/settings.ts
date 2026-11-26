import type { CrewRole } from './crew'
// The extension is required by `node --test`, which strips types without
// resolving extensionless specifiers; everything else here is bundled by Vite.
import { migrateSchedules } from './scheduler.ts'

export const AGENTS = {
  nova: { name: 'Nova', setup: 'nova init', models: 'provider:model' },
  pi: { name: 'Pi Agent', setup: 'pi auth login', models: 'provider/model' },
  claude: { name: 'Claude Code', setup: 'claude auth login', models: 'Model ID or alias' },
  codex: { name: 'Codex', setup: 'codex login', models: 'Model ID' },
  opencode: { name: 'OpenCode', setup: 'opencode auth login', models: 'provider/model' },
  commandcode: { name: 'CommandCode', setup: 'commandcode login', models: 'Model ID' },
  hermes: { name: 'Hermes', setup: 'hermes setup', models: 'Model ID or alias' },
} as const

export type AgentId = keyof typeof AGENTS
export type ModelSelection = { agent: AgentId; model: string | null; effort?: string }
export type AgentStatus = {
  id: AgentId
  name: string
  installed: boolean
  version?: string
  latest?: string
  updateAvailable?: boolean
  executable?: string
}

export const ACCENTS = {
  Nova: ['#f7b98a', '#9e4a52'],
  Sky: ['#9dccfa', '#235680'],
  Classic: ['#f7cc66', '#77500d'],
  Blue: ['#92afff', '#244987'],
  Green: ['#78d5af', '#145e45'],
  Purple: ['#c7adff', '#533588'],
  Pink: ['#ffa5c6', '#882e54'],
  Orange: ['#ffb381', '#8a3f0e'],
  Teal: ['#74d5dc', '#155b62'],
} as const

export type ColorTheme = {
  id: string
  title: string
  mode: 'light' | 'dark'
  colors: Record<string, string>
}

/// How a schedule repeats. `afterThread` and `chain` are event-driven: they have
/// no clock time of their own, which is why `at` may be 0 for them.
export type ScheduleSpec =
  | { kind: 'once' }
  | { kind: 'daily'; weekdays: boolean }
  | { kind: 'every'; minutes: number }
  | { kind: 'afterThread' }
  | { kind: 'chain'; steps: string[] }

export type Schedule = {
  id: string
  prompt: string
  workspace: string
  agent: AgentId
  /// Epoch ms of the next run; 0 until an event-driven spec is armed.
  at: number
  /// The v1 field, kept written so an older build still reads a schedule.
  daily: boolean
  spec: ScheduleSpec
  /// Who made it. Agent-made runs pass the approval and limit gates.
  origin: 'user' | 'agent' | 'preset'
  createdAt: number
  /// Progress through a chain; 0 for every other spec.
  chainStep: number
}

/// One line of the Schedules activity log.
export type ScheduleRun = {
  id: string
  scheduleId: string
  at: number
  status: 'started' | 'queued' | 'approved' | 'denied' | 'skipped' | 'failed'
  detail: string
}

/// A folder the user opened, newest first. Removing one forgets the entry; it
/// never deletes the folder or revokes workspace trust.
export type Project = { root: string; name: string; openedAt: number }

/// Voice settings. "destination: off" is the default: nothing is recorded until
/// the user turns dictation on, and the model has to be downloaded explicitly.
export type VoicePreferences = {
  destination: 'off' | 'local'
  model: string
  microphone: string
  language: string
  shortcut: 'toggle' | 'hold'
}

/// How the @team pipeline runs. `planApproval` pauses the run after Nova's
/// planning stage until the user approves, edits or skips the plan.
export type TeamPreferences = {
  planApproval: boolean
  /// How many times Ralph may re-run a genuinely failing check.
  ralphIterations: number
  /// Minutes a stage may stay silent before the watchdog fails it.
  stageTimeoutMin: number
  /// Roles included in the pipeline; the final reporter is always included.
  roles: Partial<Record<CrewRole, boolean>>
}

export type Preferences = {
  appearance: 'system' | 'light' | 'dark'
  accent: keyof typeof ACCENTS | 'Custom'
  /// One custom colour per scheme: a dark-only pick would be unreadable after
  /// switching to light, and the old single colour silently kept the dark one.
  customAccent: string
  customAccentLight: string
  /// Recently picked colours, newest first, so a good one is a click away.
  recentAccents: string[]
  /// Themes carry their own accent; turn this off to keep yours.
  accentFromTheme: boolean
  wallpaper: boolean
  still: boolean
  themes: ColorTheme[]
  themeId: string
  defaultAgent: AgentId
  models: Partial<Record<AgentId, string>>
  recentModels: ModelSelection[]
  crew: Partial<Record<CrewRole, { agent: AgentId; model?: string; systemPrompt?: string }>>
  routes: Record<string, AgentId>
  checks: Record<string, string[]>
  trustedWorkspaces: string[]
  schedules: Schedule[]
  speak: boolean
  voice: VoicePreferences
  team: TeamPreferences
  restoreFolder: boolean
  markdown: boolean
  /// The public GitHub profile the pane browses while signed out, remembered so
  /// reopening the pane does not ask for it again.
  githubLogin: string
  /// The folder the last clone went into, so the next one is one click.
  cloneRoot: string
  /// Repository full name to local path, newest last: the pane offers Open
  /// instead of Clone for anything already on this computer.
  gitHubClones: Record<string, string>
  /// What the About page does when it opens. Even "automatic" only *checks*:
  /// Nova ships no updater, so it never installs or restarts itself.
  updateMode: 'automatic' | 'manual' | 'off'
  /// Epoch ms of the last successful check, so "checked 2m ago" survives a reopen.
  updateCheckedAt: number

  // --- General -------------------------------------------------------
  /// Interface scale in percent. CSS `zoom` applies it; see `applyAppearance`.
  interfaceSize: number
  compactConversation: boolean
  expandToolActivity: boolean
  quickLauncher: boolean
  /// Accelerator string for the global chord, empty until one is saved.
  quickLauncherShortcut: string
  widgetEnabled: boolean
  widgetPinned: boolean
  restartAfterCrash: boolean
  crashReports: boolean
  keepDrafts: boolean
  projects: Project[]

  // --- Schedules -----------------------------------------------------
  /// Agent-origin wake-ups are dropped when this is off.
  agentsMaySchedule: boolean
  /// Non-user cross-thread starts wait for a human decision.
  askBeforeCrossThread: boolean
  pauseSchedules: boolean
  /// Concurrent agent-originated runs plus queued wake-ups one task may hold.
  agentLimit: number
  scheduleActivity: ScheduleRun[]
}

const defaults: Preferences = {
  appearance: 'system', accent: 'Nova', customAccent: '#b891f7', customAccentLight: '#7a2f3a',
  recentAccents: [], accentFromTheme: true,
  wallpaper: false, still: false, themes: [], themeId: '',
  defaultAgent: 'nova', models: {}, recentModels: [], crew: {}, routes: {}, checks: {}, schedules: [], trustedWorkspaces: [],
  speak: false, restoreFolder: true, markdown: true,
  githubLogin: '', cloneRoot: '', gitHubClones: {},
  voice: { destination: 'off', model: '', microphone: 'default', language: 'all', shortcut: 'toggle' },
  team: { planApproval: false, ralphIterations: 3, stageTimeoutMin: 5, roles: {} },
  updateMode: 'automatic', updateCheckedAt: 0,
  interfaceSize: 100, compactConversation: false, expandToolActivity: false,
  // Not Ctrl+Shift+Space: that is the dictation chord, and a global registration
  // would steal it from every application.
  quickLauncher: false, quickLauncherShortcut: 'CommandOrControl+Shift+N',
  widgetEnabled: false, widgetPinned: false,
  restartAfterCrash: false, crashReports: true, keepDrafts: false, projects: [],
  agentsMaySchedule: false, askBeforeCrossThread: true, pauseSchedules: false,
  agentLimit: 10, scheduleActivity: [],
}

export const INTERFACE_MIN = 50
export const INTERFACE_MAX = 200

/// Numeric preference guard: a stored blob with a junk value falls back.
function clampRange(value: unknown, min: number, max: number, fallback: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback
  return Math.min(max, Math.max(min, Math.round(value)))
}

/// Clamped so a hand-edited storage blob cannot scale the whole window to 1%.
export function clampInterfaceSize(size: number): number {
  if (!Number.isFinite(size)) return defaults.interfaceSize
  return Math.min(INTERFACE_MAX, Math.max(INTERFACE_MIN, Math.round(size)))
}

/// The shipped look plus four alternatives, written in the same workbench keys
/// an imported VS Code theme uses — one code path paints both, and a preset can
/// never describe a colour the importer could not.
export const PRESETS: ColorTheme[] = [
  {
    id: 'preset:phosphor', title: 'Phosphor', mode: 'dark',
    colors: {
      'editor.background': '#030e3e', 'editor.foreground': '#fad9b6',
      'sideBar.background': '#131b5d', 'sideBar.foreground': '#cbb4d2',
      'terminal.background': '#0a1215', 'terminal.foreground': '#fad9b6',
      'focusBorder': '#f7b98a', 'list.activeSelectionBackground': '#2a1a38',
      'editorWidget.border': '#3d203c', 'sideBar.border': '#3d203c',
      'scrollbarSlider.background': '#3d203c', 'errorForeground': '#b96f6e',
    },
  },
  {
    id: 'preset:midnight', title: 'Midnight', mode: 'dark',
    colors: {
      'editor.background': '#05060b', 'editor.foreground': '#cfd6e6',
      'sideBar.background': '#0b0e17', 'sideBar.foreground': '#9aa4bd',
      'terminal.background': '#01020a', 'terminal.foreground': '#dfe6f5',
      'focusBorder': '#7aa2ff', 'list.activeSelectionBackground': '#172038',
      'editorWidget.border': '#1c2333', 'sideBar.border': '#1c2333',
      'scrollbarSlider.background': '#2a3145', 'errorForeground': '#ff8b8b',
    },
  },
  {
    id: 'preset:slate', title: 'Slate', mode: 'dark',
    colors: {
      'editor.background': '#0f1419', 'editor.foreground': '#d7dee6',
      'sideBar.background': '#171d24', 'sideBar.foreground': '#9fb0bf',
      'terminal.background': '#0a0f13', 'terminal.foreground': '#e4ebf2',
      'focusBorder': '#7fd1b9', 'list.activeSelectionBackground': '#20302e',
      'editorWidget.border': '#2b3540', 'sideBar.border': '#2b3540',
      'scrollbarSlider.background': '#33404d', 'errorForeground': '#e08b7a',
    },
  },
  {
    id: 'preset:parchment', title: 'Parchment', mode: 'light',
    colors: {
      'editor.background': '#f6efe3', 'editor.foreground': '#2b2620',
      'sideBar.background': '#fbf6ec', 'sideBar.foreground': '#5d5347',
      'terminal.background': '#fffdf8', 'terminal.foreground': '#2b2620',
      'focusBorder': '#9a6b3f', 'list.activeSelectionBackground': '#efe2cc',
      'editorWidget.border': '#d8cbb4', 'sideBar.border': '#d8cbb4',
      'scrollbarSlider.background': '#b9a98d', 'errorForeground': '#a13c34',
    },
  },
  {
    id: 'preset:contrast', title: 'High contrast', mode: 'dark',
    colors: {
      'editor.background': '#000000', 'editor.foreground': '#ffffff',
      'sideBar.background': '#000000', 'sideBar.foreground': '#ffffff',
      'terminal.background': '#000000', 'terminal.foreground': '#ffffff',
      'focusBorder': '#ffe14d', 'list.activeSelectionBackground': '#2b2b00',
      'editorWidget.border': '#ffffff', 'sideBar.border': '#ffffff',
      'scrollbarSlider.background': '#808080', 'errorForeground': '#ff6666',
    },
  },
]

/// Appearance back to the shipped look. Imported and preset themes stay
/// installed; only the selection is dropped.
export function resetAppearance(_preferences: Preferences): Partial<Preferences> {
  return {
    appearance: defaults.appearance, accent: defaults.accent,
    customAccent: defaults.customAccent, customAccentLight: defaults.customAccentLight,
    accentFromTheme: defaults.accentFromTheme, themeId: '',
    wallpaper: defaults.wallpaper, still: defaults.still,
  }
}

const KEY = 'nova.desktop.settings.v1'

export function loadPreferences(): Preferences {
  try {
    return migratePreferences(JSON.parse(localStorage.getItem(KEY) ?? '{}'))
  } catch {
    return defaults
  }
}

/// Fills in what an older build did not save. Every new field has a default, and
/// the two shapes that changed (schedules, projects) are rebuilt rather than
/// trusted, so a stored blob from any previous version still loads.
export function migratePreferences(raw: unknown): Preferences {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return defaults
  const stored = raw as Partial<Preferences> & Record<string, unknown>
  const merged: Preferences = {
    ...defaults,
    ...stored,
    voice: { ...defaults.voice, ...(stored.voice ?? {}) },
    team: {
      ...defaults.team,
      ...(stored.team ?? {}),
      roles: { ...(stored.team as TeamPreferences | undefined)?.roles },
      ralphIterations: clampRange((stored.team as TeamPreferences | undefined)?.ralphIterations, 1, 5, defaults.team.ralphIterations),
      stageTimeoutMin: clampRange((stored.team as TeamPreferences | undefined)?.stageTimeoutMin, 1, 30, defaults.team.stageTimeoutMin),
    },
    interfaceSize: clampInterfaceSize(typeof stored.interfaceSize === 'number' ? stored.interfaceSize : defaults.interfaceSize),
    agentLimit: typeof stored.agentLimit === 'number'
      ? Math.min(100, Math.max(1, Math.round(stored.agentLimit)))
      : defaults.agentLimit,
    schedules: migrateSchedules(stored.schedules),
    scheduleActivity: Array.isArray(stored.scheduleActivity)
      ? stored.scheduleActivity.filter((entry): entry is ScheduleRun => Boolean(entry)
        && typeof entry === 'object' && typeof (entry as ScheduleRun).detail === 'string').slice(0, 50)
      : [],
    projects: Array.isArray(stored.projects)
      ? stored.projects.filter((entry): entry is Project => Boolean(entry)
        && typeof entry === 'object' && typeof (entry as Project).root === 'string'
        && typeof (entry as Project).name === 'string').slice(0, 12)
      : [],
  }
  return merged
}

export function savePreferences(value: Preferences) {
  localStorage.setItem(KEY, JSON.stringify(value))
}

export function recommendedAgent(preferences: Preferences, root?: string): AgentId {
  return (root && preferences.routes[root]) || preferences.defaultAgent
}

const HEX_COLOR = /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i

function fromTheme(colors: Record<string, string>, keys: string[]): string | undefined {
  return keys.map((key) => colors[key]).find((color) => HEX_COLOR.test(color ?? ''))
}

/// Which workbench keys feed each Nova token, and the value to derive when the
/// theme leaves that one out. Derived values reference other tokens, so a theme
/// that defines three colours still repaints the sidebar, dock, panel and
/// scrollbars instead of leaving the built-in navy behind.
const TOKENS: { token: string; keys: string[]; derive?: string }[] = [
  // The three primaries have no derivation: without a ground and an ink there
  // is nothing to mix from, so an empty theme falls back to the built-in rules.
  { token: '--ground', keys: ['editor.background', 'sideBar.background'] },
  { token: '--ink', keys: ['editor.foreground', 'foreground'] },
  { token: '--panel', keys: ['sideBar.background', 'editor.background'],
    derive: 'color-mix(in oklab, var(--ground) 90%, var(--ink))' },
  { token: '--screen', keys: ['terminal.background', 'editor.background'],
    derive: 'color-mix(in oklab, var(--ground) 88%, var(--panel))' },
  { token: '--screen-ink', keys: ['terminal.foreground', 'editor.foreground'], derive: 'var(--ink)' },
  { token: '--muted', keys: ['sideBar.foreground', 'editorLineNumber.foreground'],
    derive: 'color-mix(in oklab, var(--ink) 70%, transparent)' },
  { token: '--line', keys: ['sideBar.border', 'panel.border', 'editorWidget.border'],
    derive: 'color-mix(in oklab, var(--ink) 24%, transparent)' },
  { token: '--wash', keys: [], derive: 'color-mix(in oklab, var(--ink) 6%, transparent)' },
  { token: '--accent-wash', keys: ['list.activeSelectionBackground', 'editor.selectionBackground'],
    derive: 'color-mix(in oklab, var(--accent) 18%, var(--ground))' },
  { token: '--scroll-thumb', keys: ['scrollbarSlider.background'],
    derive: 'color-mix(in oklab, var(--ink) 26%, transparent)' },
  { token: '--scroll-track', keys: ['editor.background', 'sideBar.background'], derive: 'var(--ground)' },
  { token: '--proof-wash', keys: [], derive: 'color-mix(in oklab, var(--proof) 14%, transparent)' },
]

/// The theme in force, preset or imported. Presets come first so they cannot be
/// shadowed by an installed theme that happens to reuse an id.
export function findTheme(preferences: Preferences): ColorTheme | undefined {
  return [...PRESETS, ...preferences.themes].find(({ id }) => id === preferences.themeId)
}

export function applyAppearance(preferences: Preferences, systemLight: boolean) {
  const root = document.documentElement
  const theme = findTheme(preferences)
  const light = theme ? theme.mode === 'light'
    : preferences.appearance === 'system' ? systemLight : preferences.appearance === 'light'
  root.dataset.appearance = light ? 'light' : 'dark'
  root.dataset.wallpaper = String(preferences.wallpaper)
  root.dataset.still = String(preferences.still)
  const colors = theme?.colors ?? {}
  for (const { token, keys, derive } of TOKENS) {
    const value = fromTheme(colors, keys) ?? (theme ? derive : undefined)
    if (value) root.style.setProperty(token, value)
    else root.style.removeProperty(token)
  }
  const proof = fromTheme(colors, ['errorForeground'])
  if (theme && proof) root.style.setProperty('--proof', proof)
  else root.style.removeProperty('--proof')

  // Accent: the theme's own accent wins unless the user turned that off, so an
  // imported palette stays coherent instead of wearing Phosphor amber.
  const custom = light ? preferences.customAccentLight : preferences.customAccent
  const own = preferences.accent === 'Custom' && /^#[0-9a-f]{6}$/i.test(custom)
    ? custom
    : (ACCENTS[preferences.accent as keyof typeof ACCENTS] ?? ACCENTS.Nova)[light ? 1 : 0]
  const themed = ['focusBorder', 'button.background', 'activityBarBadge.background', 'textLink.foreground']
    .map((key) => colors[key])
    .find((color) => /^#[0-9a-f]{6}$/i.test(color ?? ''))
  const color = themed && preferences.accentFromTheme ? themed : own
  const rgb = [1, 3, 5].map((offset) => parseInt(color.slice(offset, offset + 2), 16) / 255)
  const [r, g, b] = rgb.map((channel) => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4)
  root.style.setProperty('--accent', color)
  root.style.setProperty('--on-accent', 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.179 ? '#101020' : '#ffffff')
  // Focus rings mix the accent into whatever surface they land on.
  root.style.setProperty('--accent-line', `color-mix(in oklab, ${color} 62%, transparent)`)

  // Interface size. CSS `zoom` is the layout scale every current WebView2 and
  // WebKitGTK build understands; no unsafe per-platform webview API and no extra
  // crate. The custom property is always written, so a host that ignores `zoom`
  // still scales #root through the rule in styles.css.
  const scale = clampInterfaceSize(preferences.interfaceSize) / 100
  root.style.setProperty('--ui-scale', String(scale))
  if (supportsZoom()) {
    root.style.setProperty('zoom', String(scale))
    root.dataset.zoom = 'native'
  } else {
    root.style.removeProperty('zoom')
    root.dataset.zoom = 'fallback'
  }
}

function supportsZoom(): boolean {
  try {
    return typeof CSS !== 'undefined' && CSS.supports('zoom', '1')
  } catch {
    return false
  }
}
