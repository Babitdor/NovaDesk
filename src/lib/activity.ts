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
    .replace(/_/g, ' ').replace(/\b(?:eval)\b/gi, 'Run code')
    .replace(/\s+/g, ' ').trim()
}
