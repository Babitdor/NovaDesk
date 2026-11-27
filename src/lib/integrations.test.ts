import assert from 'node:assert/strict'
import test from 'node:test'

import { appRows, sectionCounts } from './integrations.ts'

const VOICE = [
  { id: 'whisperMedium', name: 'Whisper Medium', bytes: 832 * 1024 * 1024 },
  { id: 'breeze', name: 'Breeze-ASR-25', bytes: 1.7 * 1024 ** 3 },
]
const SPEAK = [{ id: 'en-alba', name: 'Alba', bytes: 63 * 1024 * 1024 }]

test('only what is installed appears, in catalogue order', () => {
  const rows = appRows(VOICE, ['whisperMedium'], SPEAK, [], false)
  assert.deepEqual(rows.map((row) => row.name), ['Whisper Medium'])
  assert.equal(rows[0].detail, 'Dictation model · 832 MB')
})

test('a fresh install has no rows rather than placeholder ones', () => {
  assert.deepEqual(appRows(VOICE, [], SPEAK, [], false), [])
})

test('the runtime is a row of its own once it is installed', () => {
  const rows = appRows(VOICE, [], SPEAK, ['en-alba'], true)
  assert.deepEqual(rows.map((row) => row.id), ['speak:en-alba', 'speech-runtime'])
  assert.equal(rows[0].detail, 'Speech voice · 63 MB')
})

test('gigabyte downloads are not rounded to megabytes', () => {
  const rows = appRows(VOICE, ['breeze'], [], [], false)
  assert.equal(rows[0].detail, 'Dictation model · 1.7 GB')
})

test('skills count what is handed over, and the disabled ones stay counted', () => {
  const counts = sectionCounts({
    tools: 32, servers: 6, apps: 2,
    skills: [{ enabled: true }, { enabled: false }, { enabled: true }],
  })
  assert.deepEqual(counts, { tools: 32, servers: 6, skills: 2, disabled: 1, apps: 2 })
})
