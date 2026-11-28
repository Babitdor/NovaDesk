import type { ChatItem } from './chat'

export type ActivityItem = Extract<ChatItem, { kind:'tool' | 'thinking' }>
export type TranscriptEntry = Exclude<ChatItem, ActivityItem> | { id:string; kind:'activity'; items:ActivityItem[] }

export function activityEntries(items:ChatItem[]):TranscriptEntry[] {
  const entries:TranscriptEntry[] = []
  for (const item of items) {
    const last = entries.at(-1)
    if (item.kind === 'tool' || item.kind === 'thinking') {
      if (last?.kind === 'activity') last.items.push(item)
      else entries.push({ id:item.id, kind:'activity', items:[item] })
    } else entries.push(item)
  }
  return entries
}

export function activityCounts(items:ActivityItem[]) {
  const tools = items.filter((item) => item.kind === 'tool')
  return {
    tools:tools.length,
    failed:tools.filter((item) => item.ok === false).length,
    interrupted:tools.filter((item) => item.ok === undefined && item.finished).length,
    running:tools.filter((item) => item.ok === undefined && !item.finished).length,
    thinking:items.some((item) => item.kind === 'thinking' && item.active),
  }
}

export function toolLabel(label:string) {
  return label.replace(/nova-desktop[_-]|desktop\s*[·:]\s*/gi, '')
    // Harnesses name tools in their own convention: codex sends camelCase item
    // types (fileChange, commandExecution), everyone else snake_case.
    .replace(/([a-z0-9])([A-Z])/g, (_, word:string, next:string) => word + ' ' + next.toLowerCase())
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .replace(/_/g, ' ').replace(/\b(?:eval)\b/gi, 'Run code')
    .replace(/\s+/g, ' ').trim()
}

/// Tool arguments arrive as JSON, and the interesting half of one — a diff, a
/// file body, a shell script — is usually a JSON *string* inside it. Rendered
/// raw that is a single line of escaped \n in a 220px box, so decode the string
/// case and indent the object case. Anything that is not JSON passes through.
export function formatToolDetail(text:string) {
  const head = text.trimStart().slice(0, 1)
  if (head !== '{' && head !== '[' && head !== '"') return text
  try {
    const value:unknown = JSON.parse(text)
    if (typeof value === 'string') return value
    return JSON.stringify(value, null, 2)
  } catch {
    return text
  }
}
