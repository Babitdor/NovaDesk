import assert from 'node:assert/strict'
import test from 'node:test'

import { addressedCrew, parseRetry, teamOrder } from './crew.ts'

test('addressedCrew accepts the singular, plural and all as team mode', () => {
  assert.deepEqual(addressedCrew('@team fix the build'), { mode: 'team', task: 'fix the build' })
  assert.deepEqual(addressedCrew('@teams fix the build'), { mode: 'team', task: 'fix the build' })
  assert.deepEqual(addressedCrew('@all fix the build'), { mode: 'team', task: 'fix the build' })
  assert.deepEqual(addressedCrew('@team: fix the build'), { mode: 'team', task: 'fix the build' })
})

test('addressedCrew routes single roles and rejects strangers', () => {
  assert.equal(addressedCrew('@ralph run the checks')?.mode, 'ralph')
  assert.equal(addressedCrew('@teamster fix it'), undefined)
  assert.equal(addressedCrew('email me at @team'), undefined)
})

test('parseRetry catches the marker in its tolerant forms', () => {
  const bare = parseRetry('Work done so far.\n\n[[ANOTHER_ITERATION]]')
  assert.equal(bare.retry, true)
  assert.equal(bare.clean, 'Work done so far.')
  assert.equal(bare.failingCheck, 'Work done so far.')

  const withCheck = parseRetry('Progress notes\nnpm test fails on auth\n\n[[ANOTHER_ITERATION]]')
  assert.equal(withCheck.retry, true)
  assert.equal(withCheck.clean, 'Progress notes\nnpm test fails on auth')
  assert.equal(withCheck.failingCheck, 'npm test fails on auth')

  const fenced = parseRetry('Status:\n- build ok\n- tests red\n\n```\n[[ANOTHER_ITERATION]]\n```')
  assert.equal(fenced.retry, true)

  const padded = parseRetry('Partial result\n\n[[ANOTHER_ITERATION]]   \n')
  assert.equal(padded.retry, true)
  assert.equal(padded.clean, 'Partial result')
})

test('parseRetry passes plain replies through untouched', () => {
  const clean = parseRetry('All checks pass. The work is verified.')
  assert.equal(clean.retry, false)
  assert.equal(clean.clean, 'All checks pass. The work is verified.')
  assert.equal(parseRetry('').retry, false)
  // A marker mentioned mid-prose is not a request to iterate.
  assert.equal(parseRetry('I will not emit [[ANOTHER_ITERATION]] again.').retry, false)
})

test('the pipeline keeps nova as planner and reporter', () => {
  assert.equal(teamOrder[0], 'nova')
  assert.equal(teamOrder.at(-1), 'nova')
  assert.equal(new Set(teamOrder).size, 4)
})
