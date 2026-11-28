import assert from 'node:assert/strict'
import { test } from 'node:test'
import { applyEvent } from './chat.ts'
import type { ChatItem } from './chat'
import type { CrewSpeaker } from './crew'

test('interleaved teammate previews are replaced and discarded only by their owner', () => {
  const first: CrewSpeaker = { role:'nora', stage:'one' }
  const second: CrewSpeaker = { role:'serena', stage:'two' }
  let items: ChatItem[] = []
  items = applyEvent(items, { type:'messagePreview', text:'Nora preview' }, first)
  items = applyEvent(items, { type:'messagePreview', text:'Serena preview' }, second)
  items = applyEvent(items, { type:'messageFinal', text:'Nora final', replacePreview:true, plain:false }, first)
  assert.equal(items.length, 2)
  assert.equal(items[0].kind === 'assistant' && items[0].text, 'Nora final')
  items = applyEvent(items, { type:'turnFinished', reason:'completed' }, first)
  assert.equal(items.length, 2)
  items = applyEvent(items, { type:'textDiscard' }, second)
  assert.equal(items.length, 1)
})

test('thinking streams separately from replies and tools finish without an endless running state', () => {
  let items = applyEvent([], { type:'reasoningDelta', text:'Checking ' })
  items = applyEvent(items, { type:'reasoningDelta', text:'the local server.' })
  assert.equal(items.length, 1)
  assert.equal(items[0].kind === 'thinking' && items[0].text, 'Checking the local server.')
  items = applyEvent(items, { type:'toolCall', callId:'a', name:'eval', input:'start()' })
  assert.equal(items[0].kind === 'thinking' && items[0].active, false)
  items = applyEvent(items, { type:'reasoningDelta', text:'Waiting for the process.' })
  items = applyEvent(items, { type:'processOutput', stream:'stdout', chunk:'Ready' })
  assert.equal(items[1].kind === 'tool' && items[1].stream, 'Ready')
  items = applyEvent(items, { type:'turnFinished', reason:'cancelled' })
  assert.equal(items[1].kind === 'tool' && items[1].finished, true)
  assert.equal(items[2].kind === 'thinking' && items[2].active, false)
})

test('one teammate finishing does not end another teammate’s thinking or tools', () => {
  const first:CrewSpeaker = { role:'nora', stage:'one' }
  const second:CrewSpeaker = { role:'serena', stage:'two' }
  let items = applyEvent([], { type:'reasoningDelta', text:'First' }, first)
  items = applyEvent(items, { type:'toolCall', callId:'tool', name:'eval', input:'' }, second)
  items = applyEvent(items, { type:'reasoningDelta', text:'Second' }, second)
  items = applyEvent(items, { type:'reasoningDelta', text:' continued' }, first)
  assert.equal(items.length, 3)
  assert.equal(items[0].kind === 'thinking' && items[0].text, 'First continued')
  items = applyEvent(items, { type:'turnFinished', reason:'completed' }, first)
  assert.equal(items[0].kind === 'thinking' && items[0].active, false)
  assert.equal(items[1].kind === 'tool' && items[1].finished, undefined)
  assert.equal(items[2].kind === 'thinking' && items[2].active, true)
})
