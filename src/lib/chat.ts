import type { AgentEvent } from './events'
import { CREW, type CrewMode, type CrewSpeaker } from './crew'

export type ChatItem =
  | { id: string; kind: 'user'; text: string; recipient?: CrewMode }
  | { id: string; kind: 'assistant'; text: string; provisional?: boolean; author?: string; plain?: boolean; speaker?: CrewSpeaker }
  | {
      id: string
      kind: 'tool'
      callId: string
      label: string
      input: string
      ok?: boolean
      output?: string
      /// Live stdout/stderr from a running command.
      stream?: string
    }
  | { id: string; kind: 'notice'; level: string; text: string }
  | { id: string; kind: 'error'; text: string; errorKind?: string }

let seq = 0

/// Salted per page load: restored items keep the ids they were saved with, so a
/// bare counter would hand `item-3` to two different items after a restart and
/// React would key them alike.
const salt = Math.random().toString(36).slice(2, 7)

export const nextId = () => `item-${salt}-${++seq}`

export const appendUser = (items: ChatItem[], text: string, recipient?: CrewMode): ChatItem[] => [
  ...items,
  { id: nextId(), kind: 'user', text, recipient },
]

/// Folds one backend event into the transcript. Deltas extend the trailing
/// assistant item so streaming reads as typing rather than as new bubbles.
export function applyEvent(items: ChatItem[], event: AgentEvent, speaker?: CrewSpeaker): ChatItem[] {
  switch (event.type) {
    case 'messagePreview': {
      const index = items.findLastIndex((item) => item.kind === 'assistant' && item.provisional)
      if (index < 0) return [...items, { id: nextId(), kind: 'assistant', text: event.text, provisional: true, speaker }]
      return items.map((item, at) => at === index && item.kind === 'assistant'
        ? { ...item, text: item.text + event.text } : item)
    }
    case 'messageFinal': {
      const index = event.replacePreview
        ? items.findLastIndex((item) => item.kind === 'assistant' && item.provisional)
        : -1
      const reply: ChatItem = { id: items[index]?.id ?? nextId(), kind: 'assistant', text: event.text,
        author: event.author ?? undefined, plain: event.plain, speaker }
      return index < 0 ? [...items, reply] : items.map((item, at) => at === index ? reply : item)
    }
    case 'messageDelta': {
      const last = items.at(-1)
      if (last?.kind === 'assistant' && last.speaker?.stage === speaker?.stage) {
        return [...items.slice(0, -1), { ...last, text: last.text + event.text }]
      }
      return [...items, { id: nextId(), kind: 'assistant', text: event.text, speaker }]
    }
    case 'textDiscard':
      return items.filter((item) => item.kind !== 'assistant' || !item.provisional)
    case 'toolCall':
      return [
        ...items,
        { id: nextId(), kind: 'tool', callId: speaker ? speaker.stage + event.callId : event.callId,
          label: speaker ? CREW[speaker.role].name + ' · ' + event.name : event.name, input: event.input },
      ]
    case 'toolResult':
      return items.map((item) =>
        item.kind === 'tool' && item.callId === (speaker ? speaker.stage + event.callId : event.callId)
          ? { ...item, ok: event.ok, output: event.output }
          : item,
      )
    case 'processOutput': {
      const last = items.at(-1)
      if (last?.kind !== 'tool') return items
      return [
        ...items.slice(0, -1),
        { ...last, stream: (last.stream ?? '') + event.chunk },
      ]
    }
    case 'fileEdit':
      return [
        ...items,
        { id: nextId(), kind: 'tool', callId: nextId(), label: `edit ${event.path}`, input: event.diff },
      ]
    case 'notice':
      return [...items, { id: nextId(), kind: 'notice', level: event.level, text: event.message }]
    case 'approvalRequired':
      // A pending decision is a banner, not a transcript bubble: it disappears
      // once answered and must not be scrolled past.
      return items
    case 'turnError':
      return [...items, { id: nextId(), kind: 'error', text: speaker ? CREW[speaker.role].name + ' · ' + event.message : event.message, errorKind: event.kind }]
    case 'turnFinished':
      return items.filter((item) => item.kind !== 'assistant' || !item.provisional)
    case 'turnStarted':
    case 'session':
    case 'usage':
      return items
  }
}
