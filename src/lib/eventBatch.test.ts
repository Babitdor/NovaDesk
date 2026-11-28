import assert from 'node:assert/strict'
import { test } from 'node:test'
import { batchAgentEvents } from './eventBatch.ts'
import type { AgentEvent } from './events'

test('text batches while approvals and completion flush immediately in order', () => {
  const seen: AgentEvent[] = []
  const ticks: (() => void)[] = []
  const accept = batchAgentEvents((event) => seen.push(event), (flush) => ticks.push(flush))
  accept({ type:'messagePreview', text:'one' })
  accept({ type:'messagePreview', text:' two' })
  assert.equal(ticks.length, 1)
  accept({ type:'approvalRequired', interruptId:'a', kind:'tool', payload:{} })
  assert.deepEqual(seen.map((event) => event.type), ['messagePreview','approvalRequired'])
  assert.equal((seen[0] as {text:string}).text, 'one two')
  accept({ type:'turnFinished', reason:'completed' })
  ticks.forEach((flush) => flush())
  assert.equal(seen.length, 3)
})

test('thinking batches stay separate from the answer stream', () => {
  const seen:AgentEvent[] = []
  const ticks:Array<() => void> = []
  const accept = batchAgentEvents((event) => seen.push(event), (flush) => ticks.push(flush))
  accept({ type:'reasoningDelta', text:'Check ' })
  accept({ type:'reasoningDelta', text:'the server' })
  accept({ type:'messagePreview', text:'The page is ready.' })
  accept({ type:'turnFinished', reason:'completed' })
  ticks.forEach((flush) => flush())
  assert.deepEqual(seen, [
    { type:'reasoningDelta', text:'Check the server' },
    { type:'messagePreview', text:'The page is ready.' },
    { type:'turnFinished', reason:'completed' },
  ])
})
