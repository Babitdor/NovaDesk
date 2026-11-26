import assert from 'node:assert/strict'
import test from 'node:test'

import {
  PROVIDERS, SPEECH_CHUNK, formatBytes, providerInfo, splitForSpeech, stopSpeaking, voiceMeta,
  type SpeakVoice,
} from './speak.ts'

const words = (chunks: string[]) => chunks.join(' ').split(/\s+/).filter(Boolean)

test('a short reply is one piece', () => {
  assert.deepEqual(splitForSpeech('Nova is ready.'), ['Nova is ready.'])
})

test('an empty or blank reply has nothing to read', () => {
  assert.deepEqual(splitForSpeech(''), [])
  assert.deepEqual(splitForSpeech('   \n\t '), [])
})

test('a long reply is cut into pieces the engine accepts', () => {
  const sentence = 'The build finished and every check passed, so the change is ready to review. '
  const chunks = splitForSpeech(sentence.repeat(12))
  assert.ok(chunks.length > 1, 'a long reply is more than one request')
  for (const chunk of chunks) {
    assert.ok(chunk.length <= SPEECH_CHUNK, 'a piece is ' + chunk.length + ' characters')
    assert.ok(chunk.trim().length > 0, 'a piece is never blank')
  }
  // Nothing is lost on the way through: same words, same order.
  assert.deepEqual(words(chunks), words([sentence.repeat(12)]))
})

test('a single sentence longer than the cap is wrapped on word boundaries', () => {
  const long = 'word '.repeat(200).trim() + '.'
  const chunks = splitForSpeech(long, 100)
  for (const chunk of chunks) assert.ok(chunk.length <= 100, 'a piece is ' + chunk.length + ' characters')
  for (const chunk of chunks) assert.ok(!chunk.endsWith('wor'), 'no word is cut in half: ' + chunk.slice(-6))
  assert.deepEqual(words(chunks), words([long]))
})

test('a word longer than the cap is read whole rather than truncated', () => {
  const long = 'a'.repeat(120)
  const chunks = splitForSpeech('See ' + long + '.', 40)
  const carrying = chunks.filter((chunk) => chunk.includes(long))
  assert.equal(carrying.length, 1, 'the long word is one piece, not two halves')
  assert.ok(chunks.filter((chunk) => chunk !== carrying[0]).every((chunk) => chunk.length <= 40))
})

test('a link is not cut at its own dots', () => {
  const url = 'https://example.com/voice'
  const chunks = splitForSpeech('See ' + url + ' for the list.', 320)
  assert.deepEqual(chunks, ['See ' + url + ' for the list.'])
})

test('code blocks are dropped instead of read out as punctuation', () => {
  const reply = 'Here is the fix.\n\n```ts\nconst a = 1\nconst b = 2\n```\n\nIt compiles now.'
  const chunks = splitForSpeech(reply)
  // Two short sentences stay one request — what is dropped is the code itself.
  assert.deepEqual(chunks, ['Here is the fix. It compiles now.'])
  assert.ok(!chunks.join(' ').includes('```'), 'no fence is read out')
  assert.ok(!chunks.join(' ').includes('const a'), 'no code is read out')
})

test('the default cap is the one the settings page advertises', () => {
  assert.equal(SPEECH_CHUNK, 320)
})

test('every provider in the table is complete enough to draw a row', () => {
  assert.deepEqual(PROVIDERS.map((provider) => provider.id), ['openai', 'groq', 'elevenlabs', 'custom'])
  for (const provider of PROVIDERS) {
    assert.ok(provider.name, provider.id + ' has no name')
    assert.ok(provider.note, provider.id + ' has no note')
    if (provider.id !== 'custom') assert.ok(provider.models.length > 0, provider.id + ' offers no model')
    if (provider.id === 'custom') assert.equal(provider.url, '', 'the custom server has no published address')
  }
})

test('only OpenAI takes a style instruction', () => {
  assert.deepEqual(PROVIDERS.filter((provider) => provider.instructions).map((provider) => provider.id), ['openai'])
})

test('an unknown provider falls back to the first row instead of throwing', () => {
  assert.equal(providerInfo('openai').id, 'openai')
  assert.equal(providerInfo('nope' as never).id, 'openai')
})

const voice: SpeakVoice = {
  id: 'en_US-amy-low', name: 'Amy', engine: 'piper',
  language: { id: 'en_US', name: 'English (United States)' }, gender: null,
  bytes: 63_108_690, sampleRate: 16_000, speakers: 1, licence: 'See model card',
  description: 'A calm American voice.', badges: ['lowMemory'], supported: true, unsupportedReason: null,
}

test('a card line names the language, the speakers, the rate and the size', () => {
  assert.equal(voiceMeta(voice), 'English (United States) · 1 speaker · 16 kHz · 63 MB')
  assert.equal(voiceMeta({ ...voice, speakers: 904, sampleRate: 22_050 }), 'English (United States) · 904 speakers · 22.1 kHz · 63 MB')
})

test('sizes read as sizes, not as byte counts', () => {
  assert.equal(formatBytes(0), 'unknown size')
  assert.equal(formatBytes(4164), '0 MB')
  assert.equal(formatBytes(63_108_690), '63 MB')
  assert.equal(formatBytes(22_477_236), '22 MB')
})

test('stopping when nothing is playing is not an error', () => {
  // No window in this runner: the guard is what keeps a desktop-only API out of
  // the test path, and it has to be a no-op rather than a throw.
  assert.doesNotThrow(() => stopSpeaking())
  assert.doesNotThrow(() => stopSpeaking())
})
