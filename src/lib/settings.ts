import type { CrewRole } from './crew'

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

export type Schedule = {
  id: string
  prompt: string
  workspace: string
  agent: AgentId
  at: number
  daily: boolean
}

export type Preferences = {
  appearance: 'system' | 'light' | 'dark'
  accent: keyof typeof ACCENTS | 'Custom'
  customAccent: string
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
  restoreFolder: boolean
  markdown: boolean
}

const defaults: Preferences = {
  appearance: 'system', accent: 'Nova', customAccent: '#b891f7',
  wallpaper: false, still: false, themes: [], themeId: '',
  defaultAgent: 'nova', models: {}, recentModels: [], crew: {}, routes: {}, checks: {}, schedules: [], trustedWorkspaces: [],
  speak: false, restoreFolder: true, markdown: true,
}

const KEY = 'nova.desktop.settings.v1'

export function loadPreferences(): Preferences {
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(KEY) ?? '{}') }
  } catch {
    return defaults
  }
}

export function savePreferences(value: Preferences) {
  localStorage.setItem(KEY, JSON.stringify(value))
}

export function recommendedAgent(preferences: Preferences, root?: string): AgentId {
  return (root && preferences.routes[root]) || preferences.defaultAgent
}

export function applyAppearance(preferences: Preferences, systemLight: boolean) {
  const root = document.documentElement
  const theme = preferences.themes.find(({ id }) => id === preferences.themeId)
  const light = theme ? theme.mode === 'light'
    : preferences.appearance === 'system' ? systemLight : preferences.appearance === 'light'
  root.dataset.appearance = light ? 'light' : 'dark'
  root.dataset.wallpaper = String(preferences.wallpaper)
  root.dataset.still = String(preferences.still)
  const colors = theme?.colors ?? {}
  for (const [property, keys] of Object.entries({
    '--ground': ['editor.background'],
    '--panel': ['sideBar.background', 'editor.background'],
    '--screen': ['terminal.background', 'editor.background'],
    '--ink': ['editor.foreground', 'foreground'],
    '--screen-ink': ['terminal.foreground', 'editor.foreground'],
  })) {
    const value = keys.map((key) => colors[key]).find((color) => /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(color ?? ''))
    if (value) root.style.setProperty(property, value)
    else root.style.removeProperty(property)
  }
  const color = preferences.accent === 'Custom' && /^#[0-9a-f]{6}$/i.test(preferences.customAccent)
    ? preferences.customAccent
    : (ACCENTS[preferences.accent as keyof typeof ACCENTS] ?? ACCENTS.Nova)[light ? 1 : 0]
  const rgb = [1, 3, 5].map((offset) => parseInt(color.slice(offset, offset + 2), 16) / 255)
  const [r, g, b] = rgb.map((channel) => channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4)
  root.style.setProperty('--accent', color)
  root.style.setProperty('--on-accent', 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.179 ? '#101020' : '#ffffff')
}
