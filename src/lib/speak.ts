import { Channel, invoke } from '@tauri-apps/api/core'
// The extensions are required by `node --test`, which strips types without
// resolving extensionless specifiers; everything else here is bundled by Vite.
import { createDownloads } from './downloads.ts'
import { errorText } from './ipc.ts'
import { SpeechSentences, type PlaybackState } from './speechQueue.ts'
import type { AgentEvent } from './events'
import type { Preferences, SpeechDestination, SpeechProfile, SpeechProvider } from './settings.ts'

// The preference shapes live in settings.ts, which owns migration; these are
// re-exported so a component can import everything it needs from one place.
export type { SpeechDestination, SpeechProfile, SpeechProvider }

// Speak: reading a reply aloud. Two destinations with nothing in common beyond
// this module — a Piper voice on this computer (Rust spawns piper.exe per
// utterance and hands back samples) and a hosted speech service (Rust POSTs and
// hands back the audio file). The system voice needs neither and is still the
// default, because it needs no download.

/// Which engine a voice runs on. 'piper' is a child process; 'kale' and
/// 'supertonic' are ONNX weights inside this app, so they need nothing
/// installed beside them. Kale brings a phoneme front end, Supertonic reads
/// characters and is much the larger download.
export type SpeakEngine = 'piper' | 'kale' | 'supertonic' | 'kokoro'
export type SpeakBadge = 'recommended' | 'lowMemory' | 'multiSpeaker' | 'highestQuality'

export type SpeakVoice = {
  id: string
  name: string
  engine: SpeakEngine
  language: { id: string; name: string }
  /// Null unless the voice's own model card states it: never guessed here.
  gender: string | null
  bytes: number
  sampleRate: number
  speakers: number
  licence: string
  description: string
  badges: SpeakBadge[]
  supported: boolean
  unsupportedReason: string | null
}

/// Which transfer a progress frame belongs to: the first voice installs Piper
/// first (about 22 MB), then the voice itself (about 63 MB).
export type SpeakPhase = 'runtime' | 'voice' | 'model'
export type SpeakProgress = { received: number; total: number; phase: SpeakPhase }

export type SpeechAudio = {
  /// Base64 of little-endian f32 samples when kind is "samples", otherwise the
  /// audio file the service returned.
  audio: string
  kind: 'samples' | 'mp3' | 'wav' | 'ogg'
  sampleRate: number
  ms: number
}

export type SpeechProviderInfo = {
  id: SpeechProvider
  name: string
  /// Shown, not called: Rust owns where the request actually goes.
  url: string
  models: string[]
  voices: string[]
  /// Whether the provider takes a style instruction.
  instructions: boolean
  format: string
  note: string
}

/// What the settings page offers. The request shape for each of these lives in
/// speak/service.rs, which is the copy that decides what is sent.
export const PROVIDERS: SpeechProviderInfo[] = [
  {
    id: 'openai', name: 'OpenAI', url: 'https://api.openai.com/v1/audio/speech',
    models: ['gpt-4o-mini-tts', 'tts-1', 'tts-1-hd'],
    voices: ['alloy', 'ash', 'ballad', 'coral', 'echo', 'fable', 'onyx', 'nova', 'sage', 'shimmer'],
    instructions: true, format: 'wav',
    note: 'Takes a style instruction, so "read it as a tired engineer" works.',
  },
  {
    id: 'groq', name: 'Groq', url: 'https://api.groq.com/openai/v1/audio/speech',
    models: ['playai-tts', 'playai-tts-arabic'],
    voices: ['Arista-PlayAI', 'Atlas-PlayAI', 'Basil-PlayAI', 'Briggs-PlayAI', 'Calum-PlayAI',
      'Celeste-PlayAI', 'Cheyenne-PlayAI', 'Chip-PlayAI', 'Cillian-PlayAI', 'Deedee-PlayAI',
      'Fritz-PlayAI', 'Gail-PlayAI', 'Indigo-PlayAI', 'Mamaw-PlayAI', 'Mason-PlayAI',
      'Mikail-PlayAI', 'Mitch-PlayAI', 'Nia-PlayAI', 'Quinn-PlayAI', 'Ruby-PlayAI', 'Thunder-PlayAI'],
    instructions: false, format: 'wav',
    note: "The same request shape as OpenAI, on Groq's faster endpoint.",
  },
  {
    id: 'elevenlabs', name: 'ElevenLabs', url: 'https://api.elevenlabs.io/v1/text-to-speech',
    models: ['eleven_multilingual_v2', 'eleven_turbo_v2_5', 'eleven_flash_v2_5'],
    voices: [], instructions: false, format: 'mp3',
    note: 'Paste a voice id from your ElevenLabs account; it goes in the address.',
  },
  {
    id: 'custom', name: 'Your own server', url: '',
    models: [], voices: [], instructions: false, format: 'wav',
    note: "Any server that speaks OpenAI's /v1/audio/speech shape, over https.",
  },
]

export const providerInfo = (id: SpeechProvider): SpeechProviderInfo =>
  PROVIDERS.find((provider) => provider.id === id) ?? PROVIDERS[0]

export const speechProviderIds: SpeechProvider[] = ['openai', 'groq', 'elevenlabs', 'custom']

export const speakCatalog = () => invoke<SpeakVoice[]>('speak_catalog')
export const speakInstalled = () => invoke<string[]>('speak_installed')
export const speakRuntimeReady = () => invoke<boolean>('speak_runtime_ready')

/// Same Channel shape as the dictation downloads: the handler is attached before
/// the invoke, so no early progress frame is lost.
export const downloadSpeakVoice = (id: string, onEvent: (progress: SpeakProgress) => void) => {
  const channel = new Channel<SpeakProgress>()
  channel.onmessage = onEvent
  return invoke<void>('speak_download', { id, onEvent: channel })
}

/// Progress and the transfer live outside the pane, on the same reasoning as the
/// dictation downloads: Rust keeps writing after the pane is gone, and a row
/// that forgot it would start a second writer. The first voice also installs
/// Piper, so a new transfer begins at the runtime phase.
export const speakDownloads = createDownloads<SpeakProgress>((id, onEvent) =>
  downloadSpeakVoice(id, onEvent))

export const downloadVoice = (id: string, total: number) =>
  speakDownloads.start(id, { received: 0, total, phase: 'runtime' })

export const cancelSpeakDownload = (id: string) => invoke<boolean>('speak_cancel', { id })
export const deleteSpeakVoice = (id: string) => invoke<void>('speak_delete', { id })

export const speakSynthesize = (request: {
  id?: string
  destination: 'local' | 'service'
  modelId?: string
  profile?: SpeechProfile
  text: string
  speed: number
  speaker?: number
}) => invoke<SpeechAudio>('speak_synthesize', {
  // One struct parameter on the Rust side keeps the command's signature short;
  // Tauri maps it from this single named argument.
  request: {
    id: request.id,
    destination: request.destination,
    modelId: request.modelId,
    profile: request.profile,
    text: request.text,
    speed: request.speed,
    speaker: request.speaker,
  },
})

/// Longest piece one request reads. Piper handles more, but a single short
/// request keeps the first words from waiting on a long paragraph, and the
/// services bill per request anyway.
export const SPEECH_CHUNK = 320

/// Split a reply into pieces worth sending.
///
/// Code blocks are dropped: read aloud they are punctuation soup, and a reader
/// cannot act on them anyway. Everything else is split on sentence ends and then
/// on word boundaries, so no piece exceeds `max` and no word is cut in half.
export function splitForSpeech(text: string, max = SPEECH_CHUNK): string[] {
  const withoutCode = text.replace(/```[\s\S]*?```/g, ' ')
  const clean = withoutCode.replace(/\s+/g, ' ').trim()
  if (!clean) return []
  // Split after sentence punctuation that is followed by a space, so a URL or a
  // decimal like 1.5 is not cut in half at its own dot.
  const sentences = clean.split(/(?<=[.!?])\s+/)
  const chunks: string[] = []
  let current = ''
  for (const sentence of sentences) {
    const piece = sentence.trim()
    if (!piece) continue
    const joined = current ? current + ' ' + piece : piece
    if (joined.length <= max) {
      current = joined
      continue
    }
    if (current) chunks.push(current)
    current = ''
    if (piece.length <= max) {
      current = piece
      continue
    }
    const parts = wrapWords(piece, max)
    chunks.push(...parts.slice(0, -1))
    current = parts[parts.length - 1] ?? ''
  }
  if (current) chunks.push(current)
  return chunks
}

function wrapWords(text: string, max: number): string[] {
  const parts: string[] = []
  let current = ''
  for (const word of text.split(' ')) {
    const joined = current ? current + ' ' + word : word
    if (joined.length <= max) {
      current = joined
      continue
    }
    if (current) parts.push(current)
    // A single word longer than the limit is sent as it is: cutting it would
    // change what is said. A URL read aloud beats a URL truncated.
    current = word.length <= max ? word : ''
    if (word.length > max) parts.push(word)
  }
  if (current) parts.push(current)
  return parts
}

export function formatBytes(bytes: number): string {
  if (!bytes) return 'unknown size'
  if (bytes >= 1_000_000_000) return (bytes / 1_000_000_000).toFixed(1) + ' GB'
  return Math.round(bytes / 1_000_000) + ' MB'
}

/// The line under a voice's description: language, speakers, quality, size.
export function voiceMeta(voice: SpeakVoice): string {
  const parts = [voice.language.name]
  parts.push(voice.speakers > 1 ? voice.speakers + ' speakers' : '1 speaker')
  parts.push((voice.sampleRate / 1000).toFixed(1).replace('.0', '') + ' kHz')
  parts.push(formatBytes(voice.bytes))
  return parts.join(' · ')
}

// --- playback ------------------------------------------------------------

/// A new take cancels the previous one: the token is bumped by stopSpeaking, and
/// every await checks it before playing the next piece.
let session = 0
let paused = false
let activeSynthesis: string | null = null
let releasePlayback: (() => void) | null = null
let playback: PlaybackState = { id: '', status: 'idle' }
const playbackListeners = new Set<() => void>()
export const playbackSnapshot = () => playback
export const subscribePlayback = (listener: () => void) => {
  playbackListeners.add(listener)
  return () => { playbackListeners.delete(listener) }
}
function publishPlayback(next: PlaybackState) {
  playback = next
  for (const listener of playbackListeners) listener()
}
export function pauseSpeaking() {
  paused = true
  void context?.suspend()
  element?.pause()
  if (typeof window !== 'undefined' && 'speechSynthesis' in window) window.speechSynthesis.pause()
  publishPlayback({ ...playback, status:'paused' })
}
export function resumeSpeaking() {
  paused = false
  void context?.resume()
  void element?.play().catch((error) => publishPlayback({ ...playback, status:'failed', error:errorText(error) }))
  if (typeof window !== 'undefined' && 'speechSynthesis' in window) window.speechSynthesis.resume()
  publishPlayback({ ...playback, status:'playing' })
}
async function untilResumed(token: number) {
  while (paused && token === session) await new Promise((resolve) => setTimeout(resolve, 50))
}

let context: AudioContext | null = null
let source: AudioBufferSourceNode | null = null
let element: HTMLAudioElement | null = null

function audioContext(): AudioContext {
  context ??= new AudioContext()
  return context
}

/// Stop whatever is being said, on either destination.
export function stopSpeaking() {
  session += 1
  paused = false
  releasePlayback?.()
  releasePlayback = null
  if (activeSynthesis) void invoke('speak_stop', { id:activeSynthesis }).catch(() => {})
  activeSynthesis = null
  publishPlayback({ id:'', status:'idle' })
  if (typeof window !== 'undefined' && 'speechSynthesis' in window) window.speechSynthesis.cancel()
  if (source) {
    try {
      source.stop()
    } catch {
      // Already stopped: a source that ended on its own throws here.
    }
    source = null
  }
  if (element) {
    element.pause()
    element.src = ''
    element = null
  }
}

function decodeBase64(text: string): Uint8Array {
  const binary = atob(text)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}

function toSamples(bytes: Uint8Array): Float32Array<ArrayBuffer> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  const count = Math.floor(bytes.byteLength / 4)
  const samples = new Float32Array(count)
  for (let index = 0; index < count; index += 1) samples[index] = view.getFloat32(index * 4, true)
  return samples
}

async function playSamples(samples: Float32Array<ArrayBuffer>, rate: number, token: number, started:() => void): Promise<void> {
  await untilResumed(token)
  if (token !== session) return
  const player = audioContext()
  // A context created without a gesture can start suspended, and a suspended
  // graph plays nothing at all.
  await player.resume().catch(() => {})
  const buffer = player.createBuffer(1, samples.length, rate)
  buffer.copyToChannel(samples, 0)
  const bufferSource = player.createBufferSource()
  bufferSource.buffer = buffer
  bufferSource.connect(player.destination)
  source = bufferSource
  await new Promise<void>((resolve) => {
    releasePlayback = resolve
    bufferSource.onended = () => resolve()
    bufferSource.start()
    started()
  })
  if (token === session) source = null
}

/// What the page tells the browser each container is. `samples` never reaches
/// here: those bytes become an AudioBuffer instead of a blob.
const FILE_TYPES: Record<SpeechAudio['kind'], string> = {
  samples: 'audio/wav',
  wav: 'audio/wav',
  mp3: 'audio/mpeg',
  ogg: 'audio/ogg',
}

async function playFile(bytes: Uint8Array, type: string, token: number, started:() => void): Promise<void> {
  const copy = new Uint8Array(bytes)
  const url = URL.createObjectURL(new Blob([copy], { type }))
  try {
    const audio = new Audio(url)
    element = audio
    await untilResumed(token)
    if (token !== session) return
    await audio.play()
    started()
    await new Promise<void>((resolve, reject) => {
      const finish = () => {
        window.clearInterval(check)
        audio.removeEventListener('ended', finish)
        audio.removeEventListener('error', fail)
        resolve()
      }
      const fail = () => {
        window.clearInterval(check)
        audio.removeEventListener('ended', finish)
        reject(new Error('The speech audio could not be played.'))
      }
      const check = window.setInterval(() => { if (token !== session) finish() }, 120)
      releasePlayback = finish
      audio.addEventListener('ended', finish, { once:true })
      audio.addEventListener('error', fail, { once:true })
    })
    if (token === session) element = null
  } finally {
    URL.revokeObjectURL(url)
  }
}

async function speakWithSystem(text: string, speed: number, token: number, started:() => void): Promise<void> {
  if (typeof window === 'undefined' || !('speechSynthesis' in window)) throw new Error('This build has no system voice.')
  await untilResumed(token)
  if (token !== session) return
  await new Promise<void>((resolve, reject) => {
    const utterance = new SpeechSynthesisUtterance(text)
    utterance.rate = Math.min(2, Math.max(0.5, speed))
    utterance.onstart = started
    utterance.onend = () => resolve()
    utterance.onerror = (event) => event.error === 'canceled' || event.error === 'interrupted' ? resolve() : reject(new Error(event.error))
    releasePlayback = resolve
    window.speechSynthesis.speak(utterance)
  })
}

/// One producer per turn, one synthesis ahead of playback, and one global player.
export function turnSpeech(id: string, preferences: Preferences, onError: (error: string) => void = () => {}) {
  const synthesisId = crypto.randomUUID()
  const sentences = new SpeechSentences()
  const speech = preferences.speech
  const enabled = preferences.speak && speech.destination !== 'off'
  let token = session
  let started = false
  let draining = false
  let finished = false
  let failed = false
  const queue: string[] = []
  const begun = performance.now()
  let firstAudioMs: number | undefined
  let resolveDone: (error: string | null) => void = () => {}
  const done = new Promise<string | null>((resolve) => { resolveDone = resolve })
  if (!enabled) resolveDone(null)
  const profile = speech.profiles.find((entry) => entry.id === speech.profileId)

  const prepare = async (text: string): Promise<SpeechAudio | null> => {
    if (token !== session) return null
    if (speech.destination === 'system') return null
    if (speech.destination === 'service' && !profile) throw new Error('Choose a speech service in Settings, Voice.')
    if (speech.destination === 'local' && !speech.model) throw new Error('Choose a voice in Settings, Voice.')
    activeSynthesis = synthesisId
    try {
      return await speakSynthesize({ id:synthesisId, destination:speech.destination === 'local' ? 'local' : 'service',
        modelId:speech.model, profile, text, speed:speech.speed, speaker:speech.speaker || undefined })
    } finally { if (activeSynthesis === synthesisId) activeSynthesis = null }
  }
  const drain = async () => {
    if (draining || !enabled || failed) return
    draining = true
    if (!started) { stopSpeaking(); token = session; started = true }
    publishPlayback({ id, status:paused ? 'paused' : 'loading', firstAudioMs })
    try {
      let text = queue.shift()
      const preparing = (text:string) => prepare(text).then((audio) => ({ audio, error:null }), (error:unknown) => ({ audio:null, error }))
      let prepared = text ? preparing(text) : null
      while (text && token === session) {
        const result = await prepared
        if (result?.error) throw result.error
        const audio = result?.audio
        await untilResumed(token)
        if (token !== session) break
        const next = queue.shift()
        // Attach a rejection handler immediately: prefetch may fail while audio plays.
        const ahead = next ? preparing(next) : null
        const startedAudio = () => {
          if (token !== session) return
          if (firstAudioMs === undefined) {
            firstAudioMs = Math.round(performance.now() - begun)
            performance.measure('nova:first-audio', { start:begun, end:performance.now() })
          }
          publishPlayback({ id, status:paused ? 'paused' : 'playing', firstAudioMs })
        }
        if (audio?.kind === 'samples') await playSamples(toSamples(decodeBase64(audio.audio)), audio.sampleRate || 22_050, token, startedAudio)
        else if (audio) await playFile(decodeBase64(audio.audio), FILE_TYPES[audio.kind], token, startedAudio)
        else await speakWithSystem(text, speech.speed, token, startedAudio)
        text = next
        prepared = ahead
      }
    } catch (error) {
      if (token === session) {
        failed = true
        const message = errorText(error)
        publishPlayback({ id, status:'failed', error:message, firstAudioMs })
        onError(message)
        resolveDone(message)
      }
    } finally {
      draining = false
      if (token !== session) { queue.length = 0; resolveDone(null) }
      else if (!failed && queue.length) void drain()
      else if (!failed && finished) { publishPlayback({ id, status:'idle', firstAudioMs }); resolveDone(null) }
    }
  }
  return {
    accept(event: AgentEvent) {
      if (!enabled || failed) return
      if (token !== session) { resolveDone(null); return }
      queue.push(...sentences.accept(event).flatMap((sentence) => splitForSpeech(sentence)))
      if (event.type === 'turnFinished') {
        finished = true
        if (event.reason !== 'completed') { queue.length = 0; if (started) stopSpeaking() }
      }
      if (queue.length) void drain()
      else if (finished && !draining) resolveDone(null)
    },
    done,
  }
}

export async function speakReply(text: string, preferences: Preferences, id: string = crypto.randomUUID()): Promise<string | null> {
  if (!preferences.speak || !text.trim()) return null
  const player = turnSpeech(id, preferences)
  player.accept({ type:'messageFinal', text, replacePreview:true, plain:false })
  player.accept({ type:'turnFinished', reason:'completed' })
  return player.done
}
