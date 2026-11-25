import { applyEvent, type ChatItem } from './chat'
import type { AgentEvent } from './events'
import type { AgentId } from './settings'
import type { CrewMode, CrewSpeaker } from './crew'

/// One conversation, and the only unit the shell switches between.
///
/// `items` is exactly what the transcript renders, so history and screen can
/// never disagree; `state` is the one field that is not history — it is what the
/// list stamps while a turn is in flight.
export type Chat = {
  id: string
  title: string
  createdAt: number
  items: ChatItem[]
  state: 'idle' | 'running'
  /// Pinned chats sort to the top of the rail; nothing else about them changes.
  pinned?: boolean
  workspace?: string
  agent?: AgentId
  crew?: CrewMode
  model?: string | null
  effort?: string
  connectionId?: string
  session?: string
  usage?: { input: number; output: number }
}

const KEY = 'nova.chats.v1'

/// History is a convenience, not an archive: past a few dozen chats and a few
/// hundred items a session stops being something a person scrolls, and an
/// unbounded store would only turn the save into a quota error.
export const MAX_CHATS = 25
export const MAX_ITEMS = 400

export const createChat = (): Chat => ({
  id: crypto.randomUUID(),
  title: 'New task',
  createdAt: Date.now(),
  items: [],
  state: 'idle',
})

export function taskModel(chat: Chat, configured?: string): string | undefined {
  if (chat.model !== undefined) return chat.model ?? undefined
  if (chat.agent && chat.items.length) return undefined
  return configured?.trim() || undefined
}

/// A chat is named by the first thing asked of it: that is the only title the
/// user would recognise in a list they wrote themselves.
export function titleFromPrompt(prompt: string): string {
  const line = prompt.trim().split('\n')[0]?.trim() ?? ''
  if (!line) return 'New task'
  return line.length > 48 ? `${line.slice(0, 48)}…` : line
}

/// Folds a backend event into the chat that started the turn.
///
/// Events arrive on a per-turn channel, so the chat id has to travel with the
/// subscriber — never be read from whatever the user is looking at when the
/// delta lands, or a slow turn would write into the wrong transcript.
export function routeEvent(chats: Chat[], chatId: string, event: AgentEvent, speaker?: CrewSpeaker): Chat[] {
  return chats.map((chat) => {
    if (chat.id !== chatId) return chat
    const items = applyEvent(chat.items, event, speaker)
    return {
      ...chat,
      ...(event.type === 'session' && !speaker ? { session: event.id } : {}),
      ...(event.type === 'usage' ? { usage: { input: event.input, output: event.output } } : {}),
      items: items.length > MAX_ITEMS ? items.slice(-MAX_ITEMS) : items,
      // A TurnError can arrive with no TurnFinished behind it (a turn that never
      // started), so it has to clear the stamp too.
      state: !speaker && (event.type === 'turnFinished' || event.type === 'turnError') ? 'idle' : chat.state,
    }
  })
}

/// Restores what the last run wrote. A persisted `running` is a turn that died
/// with the process, so it is normalised on the way in rather than trusted.
export function load(): Chat[] {
  try {
    const raw = localStorage.getItem(KEY)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed
      .filter(isChat)
      .slice(0, MAX_CHATS)
      .map((chat) => ({
        ...chat,
        state: 'idle' as const,
        items: chat.items.slice(-MAX_ITEMS),
      }))
  } catch {
    // Corrupt history is not a startup error: an empty list beats a blank window.
    return []
  }
}

export function save(chats: Chat[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(chats.slice(0, MAX_CHATS).map((chat) => ({
      ...chat,
      items: chat.items.filter((item) => item.kind !== 'assistant' || !item.provisional),
    }))))
  } catch {
    // Quota or a blocked store: the transcript on screen is still correct, and
    // losing the next restore is better than interrupting the turn.
  }
}

function isChat(value: unknown): value is Chat {
  if (typeof value !== 'object' || value === null) return false
  const chat = value as Chat
  return (
    typeof chat.id === 'string' &&
    typeof chat.title === 'string' &&
    typeof chat.createdAt === 'number' &&
    Array.isArray(chat.items)
  )
}
