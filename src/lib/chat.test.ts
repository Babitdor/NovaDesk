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
