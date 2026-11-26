import assert from 'node:assert/strict'
import test from 'node:test'

import { clampInterfaceSize, migratePreferences } from './settings.ts'

/// A blob written by the version that shipped before this one: no `spec`, no
/// projects, no General or Schedules preferences at all.
const LEGACY = {
  appearance: 'dark',
  accent: 'Sky',
  defaultAgent: 'claude',
  schedules: [
    { id: 'x', prompt: 'standup', workspace: 'B:\\project', agent: 'nova', at: 1_700_000_000_000, daily: true },
    { id: 'y', prompt: 'once', workspace: 'B:\\project', agent: 'nova', at: 1_700_000_100_000, daily: false },
  ],
}

test('a stored v1 blob keeps both legacy schedules and gives each a spec', () => {
  const preferences = migratePreferences(LEGACY)
  assert.equal(preferences.schedules.length, 2)
  assert.deepEqual(preferences.schedules[0].spec, { kind: 'daily', weekdays: false })
  assert.deepEqual(preferences.schedules[1].spec, { kind: 'once' })
  assert.equal(preferences.schedules[1].at, 1_700_000_100_000)
  // Everything the old build knew is preserved.
  assert.equal(preferences.appearance, 'dark')
  assert.equal(preferences.accent, 'Sky')
  assert.equal(preferences.defaultAgent, 'claude')
})

test('every new preference arrives with its default', () => {
  const preferences = migratePreferences(LEGACY)
  assert.equal(preferences.interfaceSize, 100)
  assert.equal(preferences.agentLimit, 10)
  assert.equal(preferences.quickLauncher, false)
  assert.equal(preferences.quickLauncherShortcut, 'CommandOrControl+Shift+N')
  assert.equal(preferences.keepDrafts, false)
  assert.equal(preferences.askBeforeCrossThread, true)
  assert.deepEqual(preferences.projects, [])
  assert.deepEqual(preferences.scheduleActivity, [])
})

test('out-of-range numbers are clamped rather than trusted', () => {
  assert.equal(migratePreferences({ interfaceSize: 4 }).interfaceSize, 50)
  assert.equal(migratePreferences({ interfaceSize: 900 }).interfaceSize, 200)
  assert.equal(migratePreferences({ interfaceSize: Number.NaN }).interfaceSize, 100)
  assert.equal(migratePreferences({ agentLimit: 0 }).agentLimit, 1)
  assert.equal(migratePreferences({ agentLimit: 400 }).agentLimit, 100)
  assert.equal(clampInterfaceSize(105), 105)
})

test('garbage is not a reason to fail a launch', () => {
  assert.equal(migratePreferences(null).interfaceSize, 100)
  assert.equal(migratePreferences('nope').interfaceSize, 100)
  assert.equal(migratePreferences([]).interfaceSize, 100)
  assert.deepEqual(migratePreferences({ schedules: 'nope' }).schedules, [])
})

test('a partial voice block keeps its saved fields', () => {
  const preferences = migratePreferences({ voice: { destination: 'local', model: 'tiny' } })
  assert.equal(preferences.voice.destination, 'local')
  assert.equal(preferences.voice.model, 'tiny')
  assert.equal(preferences.voice.language, 'all')
})
