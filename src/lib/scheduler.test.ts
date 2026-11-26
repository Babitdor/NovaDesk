import assert from 'node:assert/strict'
import test from 'node:test'

import { advance, describe, dueAfterThread, dueSchedules, migrateSchedules, nextRun, promptFor, withinLimit } from './scheduler.ts'
import type { Schedule, ScheduleSpec } from './settings.ts'

const MINUTE = 60_000

function at(iso: string): number {
  return new Date(iso).getTime()
}

function schedule(overrides: Partial<Schedule> & { spec: ScheduleSpec }): Schedule {
  return {
    id: 's1', prompt: 'run the tests', workspace: 'B:\\project', agent: 'nova',
    at: 0, daily: false, origin: 'user', createdAt: 0, chainStep: 0, ...overrides,
  }
}

test('a daily schedule rolls to the same clock time tomorrow', () => {
  const from = at('2026-03-02T09:30:00')
  const next = nextRun({ kind: 'daily', weekdays: false }, from)
  assert.equal(next, at('2026-03-03T09:30:00'))
})

test('weekday schedules skip the weekend', () => {
  const friday = at('2026-03-06T18:00:00')
  assert.equal(new Date(nextRun({ kind: 'daily', weekdays: true }, friday)!).getDay(), 1)
})

test('an interval resumes on the next boundary instead of replaying missed runs', () => {
  const from = at('2026-03-02T09:00:00')
  const now = from + 5 * MINUTE + 1
  const next = nextRun({ kind: 'every', minutes: 1 }, from, now)
  assert.ok(next! > now)
  assert.ok(next! - now <= MINUTE)
})

test('a once schedule never repeats, and advancing drops it', () => {
  const only = schedule({ spec: { kind: 'once' }, at: at('2026-03-02T09:00:00') })
  assert.equal(nextRun(only.spec, only.at), null)
  assert.equal(advance(only, only.at), null)
})

test('only due schedules for the open workspace fire', () => {
  const now = at('2026-03-02T10:00:00')
  const due = schedule({ id: 'a', spec: { kind: 'once' }, at: now - 1000 })
  const later = schedule({ id: 'b', spec: { kind: 'once' }, at: now + 60_000 })
  const other = schedule({ id: 'c', spec: { kind: 'once' }, at: now - 1000, workspace: 'C:\\elsewhere' })
  const queued = schedule({ id: 'd', spec: { kind: 'afterThread' }, at: now - 1000 })
  const fired = dueSchedules([later, other, queued, due], 'B:\\project', now)
  assert.deepEqual(fired.map(({ id }) => id), ['a'])
})

test('a chain is due by the clock like any other entry', () => {
  const now = at('2026-03-02T10:00:00')
  const chain = schedule({ id: 'c', spec: { kind: 'chain', steps: ['one'] }, at: now - 1 })
  assert.deepEqual(dueSchedules([chain], 'B:\\project', now).map(({ id }) => id), ['c'])
})

test('afterThread fires once a task finished after the schedule was created', () => {
  const waiting = schedule({ spec: { kind: 'afterThread' }, createdAt: 1000 })
  assert.deepEqual(dueAfterThread([waiting], 'B:\\project', 999), [])
  assert.deepEqual(dueAfterThread([waiting], 'B:\\project', 1000).map(({ id }) => id), ['s1'])
})

test('a chain steps through its prompts and then ends', () => {
  const chain = schedule({
    spec: { kind: 'chain', steps: ['one', 'two'] }, at: 1000, chainStep: 0,
  })
  assert.equal(promptFor(chain), 'one')
  const second = advance(chain, 2000)
  assert.deepEqual(second, { at: 2000 + MINUTE, chainStep: 1 })
  assert.equal(promptFor({ ...chain, chainStep: 1 }), 'two')
  assert.equal(advance({ ...chain, chainStep: 1 }, 2000), null)
})

test('the agent limit counts running work and the queue together', () => {
  assert.equal(withinLimit(0, 0, 1), true)
  assert.equal(withinLimit(1, 0, 1), false)
  assert.equal(withinLimit(0, 1, 1), false)
  assert.equal(withinLimit(3, 4, 10), true)
  // Out-of-range values are clamped rather than trusted.
  assert.equal(withinLimit(0, 0, 0), true)
  assert.equal(withinLimit(0, 0, 500), true)
})

test('legacy {at,daily} entries migrate with a spec', () => {
  const legacy = [
    { id: 'x', prompt: 'standup', workspace: 'B:\\project', agent: 'nova', at: 1000, daily: true },
    { id: 'y', prompt: 'once', workspace: 'B:\\project', agent: 'nova', at: 2000, daily: false },
  ]
  const [daily, once] = migrateSchedules(legacy)
  assert.deepEqual(daily.spec, { kind: 'daily', weekdays: false })
  assert.equal(daily.daily, true)
  assert.deepEqual(once.spec, { kind: 'once' })
  assert.equal(once.at, 2000)
  assert.equal(migrateSchedules('nope').length, 0)
  assert.equal(migrateSchedules([{ prompt: '', workspace: 'x' }]).length, 0)
})

test('describe names the shape without inventing state', () => {
  assert.match(describe(schedule({ spec: { kind: 'every', minutes: 30 } })), /Every 30 min/)
  assert.match(describe(schedule({ spec: { kind: 'afterThread' } })), /After the running task/)
})
