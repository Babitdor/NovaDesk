import novaCommands from './nova-commands.json'

export type SlashCommand = {
  name: string
  canonical: string
  description: string
  surface: 'chat' | 'terminal' | 'desktop'
}

const desktopCommands: [string, string, string][] = [
  ['help', 'help', 'Browse available slash commands'],
  ['?', 'help', 'Browse available slash commands'],
  ['new', 'new', 'Start a new task'],
  ['clear', 'new', 'Start fresh and keep the current task in history'],
  ['stop', 'stop', 'Stop the running turn'],
  ['settings', 'settings', 'Open Desktop settings'],
  ['model', 'model', 'Choose the agent and model for this task'],
  ['models', 'model', 'Choose the agent and model for this task'],
  ['auth', 'auth', 'Open provider setup'],
  ['connect', 'auth', 'Open provider setup'],
  ['mcp', 'mcp', 'Open MCP connections'],
  ['theme', 'theme', 'Open appearance settings'],
  ['themes', 'theme', 'Open appearance settings'],
  ['terminal', 'terminal', 'Open the workspace PowerShell dock'],
]

export const desktopSlashCommands: SlashCommand[] = desktopCommands.map(([name, canonical, description]) =>
  ({ name, canonical, description, surface: 'desktop' }))

const desktopNames = new Set(desktopSlashCommands.map(({ name }) => name))

export const novaSlashCommands: SlashCommand[] = novaCommands.flatMap(([name, canonical, description, surface]) =>
  desktopNames.has(name) || (surface !== 'chat' && surface !== 'terminal') ? []
    : [{ name, canonical, description, surface }])

export const allNovaSlashCommands = [...desktopSlashCommands, ...novaSlashCommands]
