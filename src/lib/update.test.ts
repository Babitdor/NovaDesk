import assert from 'node:assert/strict'
import test from 'node:test'

import { progressLabel, progressPercent } from './update.ts'
import type { UpdateProgress } from './ipc.ts'

const MB = 1024 * 1024

function downloading(downloaded: number, total: number | null): UpdateProgress {
  return { stage: 'downloading', downloaded, total }
}

test('a percent needs a total the server actually sent', () => {
  assert.equal(progressPercent(null), null)
  assert.equal(progressPercent({ stage: 'checking', downloaded: 0, total: null }), null)
  assert.equal(progressPercent(downloading(MB, null)), null)
  // 0 is "no denominator", not "0%"."
  assert.equal(progressPercent(downloading(0, 0)), null)
})

test('the percent measures the bytes Rust summed against the advertised size', () => {
  assert.equal(progressPercent(downloading(0, 4 * MB)), 0)
  assert.equal(progressPercent(downloading(2 * MB, 4 * MB)), 50)
  assert.equal(progressPercent(downloading(4 * MB, 4 * MB)), 100)
})

test('more bytes than advertised never reads past 100%', () => {
  assert.equal(progressPercent(downloading(5 * MB, 4 * MB)), 100)
})

test('the label names the size, and the percent only when there is a total', () => {
  assert.match(progressLabel({ stage: 'checking', downloaded: 0, total: null }), /Checking GitHub/)
  const half = progressLabel(downloading(2 * MB, 4 * MB))
  assert.match(half, /2\.0 MB of 4\.0 MB/)
  assert.match(half, /50%/)
  const unknown = progressLabel(downloading(3 * MB, null))
  assert.match(unknown, /Downloading 3\.0 MB/)
  assert.equal(unknown.includes('%'), false)
})

test('installing says the window closes, and no progress reads as an empty line', () => {
  assert.match(progressLabel({ stage: 'installing', downloaded: 4 * MB, total: 4 * MB }), /closes and reopens/)
  assert.equal(progressLabel(null), '')
})
