import type { AgentEvent } from './events'

export function batchAgentEvents(deliver: (event: AgentEvent) => void, schedule: (flush: () => void) => void) {
  let queued: Extract<AgentEvent, { type:'messageDelta' | 'messagePreview' | 'finalAnswerDelta' }> | null = null
  let scheduled = false
  const flush = () => {
    scheduled = false
    const event = queued
    queued = null
    if (event) deliver(event)
  }
  return (event: AgentEvent) => {
    if (event.type !== 'messageDelta' && event.type !== 'messagePreview' && event.type !== 'finalAnswerDelta') {
      flush()
      deliver(event)
      return
    }
    if (queued && queued.type !== event.type) flush()
    queued = queued ? { ...queued, text:queued.text + event.text } : { ...event }
    if (!scheduled) { scheduled = true; schedule(flush) }
  }
}
