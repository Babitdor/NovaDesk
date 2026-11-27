import assert from 'node:assert/strict'
import test from 'node:test'

import { createDownloads } from './downloads.ts'

type Frame = { done: number }

// A transfer the test drives by hand: the returned promise settles only when the
// test says so, which is what a real download looks like to the pane.
function controlled() {
  let emit: (frame: Frame) => void = () => {}
  let settle: () => void = () => {}
  let fail: (error: Error) => void = () => {}
  const transfer = (_id: string, onEvent: (frame: Frame) => void) => {
    emit = onEvent
    return new Promise<void>((resolve, reject) => { settle = resolve; fail = reject })
  }
  return { transfer, emit: (f: Frame) => emit(f), settle: () => settle(), fail: (e: Error) => fail(e) }
}

test('a transfer is visible the moment it starts, before any progress', async () => {
  const hand = controlled()
  const downloads = createDownloads<Frame>(hand.transfer)
  const running = downloads.start('whisper-base', { done: 0 })
  assert.deepEqual([...downloads.snapshot()], [['whisper-base', { done: 0 }]])
  hand.settle()
  await running
})

test('each frame replaces the map instead of mutating it', async () => {
  const hand = controlled()
  const downloads = createDownloads<Frame>(hand.transfer)
  const before = downloads.snapshot()
  const running = downloads.start('whisper-base', { done: 0 })
  const started = downloads.snapshot()
  assert.notEqual(started, before, 'React needs a new identity to re-render')
  hand.emit({ done: 5 })
  assert.equal(downloads.snapshot().get('whisper-base')?.done, 5)
  assert.equal(started.get('whisper-base')?.done, 0, 'the old snapshot is left alone')
  hand.settle()
  await running
})

test('the row is gone once the transfer ends, and every listener hears about it', async () => {
  const hand = controlled()
  const downloads = createDownloads<Frame>(hand.transfer)
  let calls = 0
  const stop = downloads.subscribe(() => { calls++ })
  const running = downloads.start('whisper-base', { done: 0 })
  hand.emit({ done: 5 })
  hand.settle()
  await running
  assert.equal(downloads.snapshot().size, 0)
  assert.equal(calls, 3, 'start, one frame, and the cleanup')
  stop()
  const again = downloads.start('whisper-base', { done: 0 })
  hand.settle()
  await again
  assert.equal(calls, 3, 'an unsubscribed pane is not told again')
})

test('a failed transfer clears its row rather than leaving it downloading', async () => {
  const hand = controlled()
  const downloads = createDownloads<Frame>(hand.transfer)
  const running = downloads.start('whisper-base', { done: 0 })
  hand.fail(new Error('the disk filled up'))
  await assert.rejects(running, /disk filled up/)
  assert.equal(downloads.snapshot().size, 0)
})

test('two models download side by side without clobbering each other', async () => {
  const transfers = new Map<string, (frame: Frame) => void>()
  const settles = new Map<string, () => void>()
  const downloads = createDownloads<Frame>((id, onEvent) => {
    transfers.set(id, onEvent)
    return new Promise<void>((resolve) => { settles.set(id, resolve) })
  })
  const first = downloads.start('whisper-base', { done: 0 })
  const second = downloads.start('whisper-small', { done: 0 })
  transfers.get('whisper-base')?.({ done: 1 })
  assert.equal(downloads.snapshot().size, 2)
  assert.equal(downloads.snapshot().get('whisper-small')?.done, 0)
  settles.get('whisper-base')?.()
  await first
  assert.deepEqual([...downloads.snapshot().keys()], ['whisper-small'], 'only the finished one leaves')
  settles.get('whisper-small')?.()
  await second
})
