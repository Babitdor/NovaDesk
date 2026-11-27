import assert from 'node:assert/strict'
import { test } from 'node:test'
import { loadPreferences } from './settings.ts'
import { pauseSpeaking, playbackSnapshot, resumeSpeaking, speakReply, stopSpeaking, turnSpeech } from './speak.ts'

const tick = () => new Promise((resolve) => setTimeout(resolve, 0))

test('off resolves immediately and stopped producers cannot start on late final text', async () => {
  const preferences = loadPreferences()
  assert.equal(await speakReply('A reply.', { ...preferences, speak:true, speech:{ ...preferences.speech, destination:'off' } }), null)
  const producer = turnSpeech('late', { ...preferences, speak:true })
  producer.accept({ type:'finalAnswerDelta', text:'Incomplete' })
  stopSpeaking()
  producer.accept({ type:'finalAnswerDelta', text:'. Finished. ' })
  producer.accept({ type:'turnFinished', reason:'completed' })
  assert.equal(await producer.done, null)
  assert.equal(playbackSnapshot().status, 'idle')
})

test('playback prefetches one chunk and pause/resume does not advance the queue', async () => {
  const calls: string[] = []
  const players: EventTarget[] = []
  const originalWindow = globalThis.window
  const originalAudio = globalThis.Audio
  class AudioFixture extends EventTarget {
    src = ''
    constructor() { super(); players.push(this) }
    play() { return Promise.resolve() }
    pause() {}
  }
  Object.assign(globalThis, { Audio:AudioFixture, window:{ setInterval, clearInterval,
    __TAURI_INTERNALS__:{ invoke:async (command: string, { request }: { request?:{text:string} }) => {
      if (command === 'speak_synthesize') { calls.push(request!.text); return { kind:'wav', audio:'AAAA', sampleRate:0, ms:1 } }
    } },
  } })
  try {
    const preferences = loadPreferences()
    const done = speakReply('One sentence. Two sentences. Three sentences.', { ...preferences, speak:true,
      speech:{ ...preferences.speech, destination:'local', model:'fixture' } })
    await tick()
    assert.deepEqual(calls, ['One sentence.','Two sentences.'])
    pauseSpeaking()
    assert.equal(playbackSnapshot().status, 'paused')
    await tick()
    assert.equal(players.length, 1)
    resumeSpeaking()
    players[0].dispatchEvent(new Event('ended'))
    await tick()
    assert.equal(calls.length, 3)
    assert.equal(players.length, 2)
    stopSpeaking()
    assert.equal(await done, null)
    assert.equal(playbackSnapshot().status, 'idle')
  } finally { stopSpeaking(); Object.assign(globalThis, { window:originalWindow, Audio:originalAudio }) }
})

test('synthesis failure is visible and does not continue preparing chunks', async () => {
  const originalWindow = globalThis.window
  let calls = 0
  Object.assign(globalThis, { window:{ __TAURI_INTERNALS__:{ invoke:async () => { calls++; throw new Error('fixture synthesis failed') } } } })
  try {
    const preferences = loadPreferences()
    const error = await speakReply('One sentence. Two sentences.', { ...preferences, speak:true,
      speech:{ ...preferences.speech, destination:'local', model:'fixture' } })
    assert.match(error!, /fixture synthesis failed/)
    assert.equal(playbackSnapshot().status, 'failed')
    assert.equal(calls, 1)
  } finally { stopSpeaking(); Object.assign(globalThis, { window:originalWindow }) }
})


test('stopping playback absorbs a failed pending prefetch', async () => {
  const originalWindow = globalThis.window
  const originalAudio = globalThis.Audio
  let rejectPrefetch: (error: Error) => void = () => {}
  let calls = 0
  class AudioFixture extends EventTarget {
    src = ''
    play() { return Promise.resolve() }
    pause() {}
  }
  Object.assign(globalThis, { Audio:AudioFixture, window:{ setInterval, clearInterval,
    __TAURI_INTERNALS__:{ invoke:async (command:string) => {
      if (command !== 'speak_synthesize') return
      if (++calls === 1) return { kind:'wav', audio:'AAAA', sampleRate:0, ms:1 }
      return new Promise((_, reject) => { rejectPrefetch = reject })
    } },
  } })
  try {
    const preferences = loadPreferences()
    const done = speakReply('One sentence. Two sentences.', { ...preferences, speak:true,
      speech:{ ...preferences.speech, destination:'local', model:'fixture' } })
    await tick()
    assert.equal(calls, 2)
    stopSpeaking()
    rejectPrefetch(new Error('cancelled pending synthesis'))
    assert.equal(await done, null)
    await tick()
    assert.equal(playbackSnapshot().status, 'idle')
  } finally { stopSpeaking(); Object.assign(globalThis, { window:originalWindow, Audio:originalAudio }) }
})
