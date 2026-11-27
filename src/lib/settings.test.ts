import assert from 'node:assert/strict'
import test from 'node:test'

import { clampInterfaceSize, clampSpeechSpeed, migratePreferences } from './settings.ts'

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
  assert.deepEqual(preferences.disabledSkills, [])
})

test('a saved skill switch list is kept, and anything that is not a name is dropped', () => {
  assert.deepEqual(migratePreferences({ disabledSkills: ['adhd', 'awwwards'] }).disabledSkills,
    ['adhd', 'awwwards'])
  // A blob written by something else: only the strings can name a skill folder.
  assert.deepEqual(migratePreferences({ disabledSkills: ['adhd', 7, null, {}] }).disabledSkills,
    ['adhd'])
  assert.deepEqual(migratePreferences({ disabledSkills: 'adhd' }).disabledSkills, [])
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

test('a saved transcription service survives a reload', () => {
  const preferences = migratePreferences({
    voice: {
      destination: 'service',
      profiles: [{ id: 'p1', name: 'Work', provider: 'mistral', model: 'voxtral-mini-latest', url: '', key: 'k' }],
      profileId: 'p1',
    },
  })
  assert.equal(preferences.voice.destination, 'service')
  assert.equal(preferences.voice.profileId, 'p1')
  assert.deepEqual(preferences.voice.profiles, [
    { id: 'p1', name: 'Work', provider: 'mistral', model: 'voxtral-mini-latest', url: '', key: 'k' },
  ])
})

test('a voice destination this build does not have turns dictation off', () => {
  assert.equal(migratePreferences({ voice: { destination: 'cloud' } }).voice.destination, 'off')
  assert.equal(migratePreferences({ voice: { destination: 'service' } }).voice.destination, 'off')
})

test('a profile with an unknown provider or no id is dropped, not kept half-read', () => {
  const preferences = migratePreferences({
    voice: {
      destination: 'service',
      profiles: [
        { id: 'ok', provider: 'openai', model: 'whisper-1', key: 'k' },
        { id: '', provider: 'openai' },
        { id: 'x', provider: 'elevenlabs' },
        'nope',
      ],
    },
  })
  assert.deepEqual(preferences.voice.profiles.map((entry) => entry.id), ['ok'])
  // The saved selection pointed at nothing that survived, so dictation is off.
  assert.equal(preferences.voice.profileId, 'ok')
  assert.equal(preferences.voice.destination, 'service')
})

test('a build that never saved speech reads the system voice', () => {
  const preferences = migratePreferences(LEGACY)
  assert.equal(preferences.speech.destination, 'system')
  assert.equal(preferences.speech.speed, 1)
  assert.equal(preferences.speech.profiles.length, 0)
  assert.equal(preferences.speech.profileId, '')
  // The old master switch still means what it meant.
  assert.equal(preferences.speak, false)
})

test('a speech destination this build does not have falls back to the system voice', () => {
  const preferences = migratePreferences({ speech: { destination: 'cloud' } })
  assert.equal(preferences.speech.destination, 'system')
})

test('speed is clamped to what the engine accepts', () => {
  assert.equal(migratePreferences({ speech: { speed: 9 } }).speech.speed, 2)
  assert.equal(migratePreferences({ speech: { speed: 0 } }).speech.speed, 0.5)
  assert.equal(migratePreferences({ speech: { speed: 1.26 } }).speech.speed, 1.3)
  assert.equal(migratePreferences({ speech: { speed: 'fast' } }).speech.speed, 1)
  assert.equal(clampSpeechSpeed(Number.NaN), 1)
})

test('a profile missing its id or provider is dropped, a whole one is kept', () => {
  const preferences = migratePreferences({
    speech: {
      destination: 'service',
      profiles: [
        { id: 'a', name: 'Mine', provider: 'custom', model: 'm', voice: 'v', url: 'https://x.test/t', key: 'k', instructions: '' },
        { name: 'no id', provider: 'openai' },
        { id: 'b', provider: 'nope' },
        'junk',
        null,
        { id: 'c', provider: 'groq' },
      ],
      profileId: 'a',
    },
  })
  assert.deepEqual(preferences.speech.profiles.map((profile) => profile.id), ['a', 'c'])
  assert.equal(preferences.speech.profileId, 'a')
  // Missing fields become empty strings rather than undefined.
  assert.equal(preferences.speech.profiles[1].key, '')
  assert.equal(preferences.speech.profiles[1].name, 'Speech service')
})

test('a profile id that is not in the list is replaced by the first saved one', () => {
  const preferences = migratePreferences({
    speech: {
      profiles: [{ id: 'kept', name: 'Kept', provider: 'openai' }],
      profileId: 'gone',
    },
  })
  assert.equal(preferences.speech.profileId, 'kept')
})

test('no saved profile means no profile id at all', () => {
  const preferences = migratePreferences({ speech: { profileId: 'ghost', profiles: 'nope' } })
  assert.equal(preferences.speech.profileId, '')
  assert.deepEqual(preferences.speech.profiles, [])
})

/// Auto-approve widens what runs without asking, so the stored blob may only
/// ever say one of the three strings the page offers.
test('auto-approve is off unless the blob names an option', () => {
  assert.equal(migratePreferences({}).autoApprove, false)
  assert.equal(migratePreferences({ autoApprove: 'tools' }).autoApprove, false)
  assert.equal(migratePreferences({ autoApprove: 'all' }).autoApprove, true)
  assert.equal(migratePreferences({ autoApprove: 'everything' }).autoApprove, false)
  assert.equal(migratePreferences({ autoApprove: true }).autoApprove, true)
  assert.equal(migratePreferences({ autoApprove: 'ALL' }).autoApprove, false)
})

test('a speaker number is a whole number inside the range the page offers', () => {
  assert.equal(migratePreferences({ speech: { speaker: 904 } }).speech.speaker, 904)
  assert.equal(migratePreferences({ speech: { speaker: -3 } }).speech.speaker, 0)
  assert.equal(migratePreferences({ speech: { speaker: 12.6 } }).speech.speaker, 13)
  assert.equal(migratePreferences({ speech: { speaker: 'x' } }).speech.speaker, 0)
})

 test('tools-only migration explains why approval was disabled', () => {
  assert.equal(migratePreferences({ autoApprove: 'tools' }).approvalMigrationNotice, true)
  assert.equal(migratePreferences({ autoApprove: false, approvalMigrationNotice: false }).approvalMigrationNotice, false)
})
