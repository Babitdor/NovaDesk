import type { AgentEvent } from './events'
import { CREW, type CrewMode, type CrewSpeaker } from './crew.ts'

export type ChatItem =
  | { id: string; kind: 'user'; text: string; recipient?: CrewMode }
  | { id: string; kind: 'thinking'; text: string; active: boolean; speaker?: CrewSpeaker }
  | { id: string; kind: 'assistant'; text: string; provisional?: boolean; author?: string; plain?: boolean; speaker?: CrewSpeaker }
  | {
      id: string
      kind: 'tool'
      callId: string
      label: string
      input: string
      ok?: boolean
      finished?: boolean
      speaker?: CrewSpeaker
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
  if (['toolCall', 'messageDelta', 'messagePreview', 'finalAnswerDelta', 'messageFinal', 'turnFinished', 'turnError'].includes(event.type)) {
    items = items.map((item) => item.kind === 'thinking' && item.active && item.speaker?.stage === speaker?.stage
      ? { ...item, active:false } : item)
  }
  if (event.type === 'turnFinished' || event.type === 'turnError') {
    items = items.map((item) => item.kind === 'tool' && item.ok === undefined && item.speaker?.stage === speaker?.stage
      ? { ...item, finished:true } : item)
  }
  switch (event.type) {
    case 'reasoningDelta': {
      if (!event.text) return items
      const index = items.findLastIndex((item) => item.kind === 'thinking' && item.active && item.speaker?.stage === speaker?.stage)
      if (index >= 0) {
        return items.map((item, at) => at === index && item.kind === 'thinking'
          ? { ...item, text:(item.text + event.text).slice(0, 64000) } : item)
      }
      return [...items, { id:nextId(), kind:'thinking', text:event.text.slice(0, 64000), active:true, speaker }]
    }
    case 'finalAnswerDelta':
    case 'messagePreview': {
      const index = items.findLastIndex((item) => item.kind === 'assistant' && item.provisional && item.speaker?.stage === speaker?.stage)
      if (index < 0) return [...items, { id: nextId(), kind: 'assistant', text: event.text, provisional: true, speaker }]
      return items.map((item, at) => at === index && item.kind === 'assistant'
        ? { ...item, text: item.text + event.text } : item)
    }
    case 'messageFinal': {
      const index = event.replacePreview
        ? items.findLastIndex((item) => item.kind === 'assistant' && item.provisional && item.speaker?.stage === speaker?.stage)
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
      return items.filter((item) => item.kind !== 'assistant' || !item.provisional || item.speaker?.stage !== speaker?.stage)
    case 'toolCall':
      return [
        ...items,
        { id: nextId(), kind: 'tool', callId: speaker ? speaker.stage + event.callId : event.callId,
          label: speaker ? CREW[speaker.role].name + ' · ' + event.name : event.name, input: event.input, speaker },
      ]
    case 'toolResult':
      return items.map((item) =>
        item.kind === 'tool' && item.callId === (speaker ? speaker.stage + event.callId : event.callId)
          ? { ...item, ok: event.ok, output: event.output }
          : item,
      )
    case 'processOutput': {
      const index = items.findLastIndex((item) => item.kind === 'tool' && item.speaker?.stage === speaker?.stage)
      return items.map((item, at) => at === index && item.kind === 'tool'
        ? { ...item, stream:(item.stream ?? '') + event.chunk } : item)
    }
    case 'fileEdit':
      return [
        ...items,
        { id: nextId(), kind: 'tool', callId: nextId(), label: `edit ${event.path}`, input: event.diff, ok:true, speaker },
      ]
    case 'subagentTask': {
      // One row per subagent dispatch, upserted by task id; the label on a row
      // keeps its own stage prefix so crew turns never collide.
      const callId = (speaker?.stage ?? '') + 'subagent:' + event.taskId
      const label = event.label
        ?? (event.subagentType ? 'Subagent · ' + event.subagentType : 'Subagent task')
      const index = items.findIndex((item) => item.kind === 'tool' && item.callId === callId)
      if (index < 0) {
        return [...items, { id: nextId(), kind: 'tool', callId, label, input: '', speaker }]
      }
      const finished = event.status !== 'running'
      const suffix = finished && event.durationMs ? ` · ${Math.round(event.durationMs / 1000)}s` : ''
      return items.map((item, at) => at === index && item.kind === 'tool'
        ? { ...item, label: label + suffix, ok: finished ? !/error|fail|cancel/i.test(event.status) : undefined }
        : item)
    }
    case 'subagentPreview': {
      const callId = (speaker?.stage ?? '') + 'subagent:' + event.taskId
      const index = items.findLastIndex((item) => item.kind === 'tool' && item.callId === callId)
      if (index < 0) return items
      return items.map((item, at) => at === index && item.kind === 'tool'
        ? { ...item, stream: (item.stream ?? '') + event.text }
        : item)
    }
    case 'notice':
      return [...items, { id: nextId(), kind: 'notice', level: event.level, text: event.message }]
    case 'approvalResolved':
    case 'approvalRequired':
      // A pending decision is a banner, not a transcript bubble: it disappears
      // once answered and must not be scrolled past.
      return items
    case 'turnError':
      return [...items, { id: nextId(), kind: 'error', text: speaker ? CREW[speaker.role].name + ' · ' + event.message : event.message, errorKind: event.kind }]
    case 'turnFinished':
      return items.filter((item) => item.kind !== 'assistant' || !item.provisional || item.speaker?.stage !== speaker?.stage)
    case 'timing':
    case 'turnStarted':
    case 'session':
    case 'usage':
      return items
    // Cross-task starts are decisions for App, not transcript entries: the
    // request is answered by a card and the outcome lands in the activity log.
    case 'wakeupRequested':
    case 'threadMessage':
      return items
  }
}
