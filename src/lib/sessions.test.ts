import assert from 'node:assert/strict'
import test from 'node:test'

import type { ChatItem } from './chat.ts'
import { createChat, forkChat } from './sessions.ts'

const entry = (id: string, kind: 'user' | 'assistant', text: string): ChatItem => ({ id, kind, text })

const parent = () => ({
  ...createChat(),
  title: 'Ship the panel',
  items: [entry('a', 'user', 'one'), entry('b', 'assistant', 'two'), entry('c', 'user', 'three')],
  session: 'sess-1',
})

test('a fork copies the transcript through the entry it was forked at', () => {
  assert.deepEqual(forkChat(parent(), 'b').items.map((item) => item.id), ['a', 'b'])
})

test('a fork is its own task, keeps the settings and pins nothing to itself', () => {
  const source = parent()
  const forked = forkChat(source, 'c')
  assert.notEqual(forked.id, source.id)
  assert.equal(forked.title, source.title)
  assert.equal(forked.session, source.session)
  assert.equal(forked.state, 'idle')
  assert.equal(forked.pinned, undefined)
  assert.equal(forked.items.length, source.items.length)
})

test('an entry that is not in the task forks the whole transcript', () => {
  const source = parent()
  assert.deepEqual(forkChat(source, 'missing').items, source.items)
})