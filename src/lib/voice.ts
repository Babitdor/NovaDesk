import { Channel, invoke } from '@tauri-apps/api/core'

// Voice: on-device dictation. The Rust side owns the catalogue, the downloads
// and inference; this module is the typed boundary plus the microphone capture
// both the settings page and the composer share.

export type VoiceEngine = 'parakeet' | 'canary' | 'cohere' | 'moonshineBase' | 'moonshineTiny' | 'senseVoice' | 'none'
export type VoiceBadge = 'recommended' | 'translate' | 'livePreview' | 'streaming'

export type VoiceLanguage = { id: string; name: string }

export type VoiceModel = {
  id: string
  name: string
  engine: VoiceEngine
  /// Languages this catalogue can name, used by the filter.
  languages: VoiceLanguage[]
  /// Languages the vendor publishes, which can be more than are named here.
  languageCount: number
  bytes: number
  licence: string
  accuracy: string | null
  speed: string | null
  description: string
  badges: VoiceBadge[]
  supported: boolean
  unsupportedReason: string | null
}

export type VoiceProgress = { received: number; total: number }
export type VoiceTranscription = { text: string; ms: number }

/// The dictation shortcut. Kept here so the page and the composer cannot drift.
export const VOICE_SHORTCUT = { key: ' ', code: 'Space', control: true, shift: true }
export const VOICE_SHORTCUT_LABEL = 'Ctrl+Shift+Space'

export const voiceCatalog = () => invoke<VoiceModel[]>('voice_catalog')
export const voiceInstalled = () => invoke<string[]>('voice_installed')
export const voiceEngineReady = () => invoke<boolean>('voice_engine_ready')

/// Same Channel shape as startTurn: the handler is attached before the invoke,
/// so no early progress frame is lost.
export const downloadVoiceModel = (id: string, onEvent: (progress: VoiceProgress) => void) => {
  const channel = new Channel<VoiceProgress>()
  channel.onmessage = onEvent
  return invoke<void>('voice_download', { id, onEvent: channel })
}

export const cancelVoiceDownload = (id: string) => invoke<boolean>('voice_cancel', { id })
export const deleteVoiceModel = (id: string) => invoke<void>('voice_delete', { id })

/// Samples go over as base64 of little-endian f32, interleaved at the rate the
/// device recorded. Rust downmixes and resamples to 16 kHz.
export const voiceTranscribe = (request: {
  modelId: string
  language?: string
  samples: Float32Array
  sampleRate: number
  channels: number
}) => invoke<VoiceTranscription>('voice_transcribe', {
  modelId: request.modelId,
  language: request.language,
  samples: encodeSamples(request.samples),
  sampleRate: request.sampleRate,
  channels: request.channels,
})

function encodeSamples(samples: Float32Array): string {
  const bytes = new Uint8Array(samples.buffer, samples.byteOffset, samples.byteLength)
  let binary = ''
  const chunk = 0x8000
  for (let start = 0; start < bytes.length; start += chunk) {
    binary += String.fromCharCode(...bytes.subarray(start, start + chunk))
  }
  return btoa(binary)
}

export function formatBytes(bytes: number): string {
  if (!bytes) return 'unknown size'
  if (bytes >= 1_000_000_000) return (bytes / 1_000_000_000).toFixed(1) + ' GB'
  return Math.round(bytes / 1_000_000) + ' MB'
}

/// The line under a model's description: grade, decode speed, coverage, size.
export function modelMeta(model: VoiceModel): string {
  const parts: string[] = []
  if (model.accuracy) parts.push('English: ' + model.accuracy)
  if (model.speed) parts.push('Speed: ' + model.speed)
  parts.push(model.languageCount === 1 ? '1 language' : model.languageCount + ' languages')
  if (model.bytes) parts.push(formatBytes(model.bytes))
  return parts.join(' · ')
}

export function languageName(model: VoiceModel, code: string): string {
  return model.languages.find((language) => language.id === code)?.name ?? code
}

/// Every language any model offers, in catalogue order, for the filter pane.
export function catalogueLanguages(models: VoiceModel[]): VoiceLanguage[] {
  const seen = new Map<string, string>()
  for (const model of models) {
    for (const language of model.languages) if (!seen.has(language.id)) seen.set(language.id, language.name)
  }
  return [...seen].map(([id, name]) => ({ id, name }))
}

export type Recording = {
  samples: Float32Array
  sampleRate: number
  channels: number
  duration: number
  /// Loudest sample in the clip, 0..1, so a silent take can be refused before
  /// it ever reaches the model.
  peak: number
}

/// Encode a take as 16-bit mono WAV. Only the microphone test needs a file to
/// play back; transcription reads the samples directly.
export function toWav(recording: Recording): Blob {
  const { samples, sampleRate } = recording
  const bytes = new ArrayBuffer(44 + samples.length * 2)
  const view = new DataView(bytes)
  const text = (offset: number, value: string) => {
    for (let index = 0; index < value.length; index += 1) view.setUint8(offset + index, value.charCodeAt(index))
  }
  text(0, 'RIFF'); view.setUint32(4, 36 + samples.length * 2, true); text(8, 'WAVE')
  text(12, 'fmt '); view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true)
  view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * 2, true)
  view.setUint16(32, 2, true); view.setUint16(34, 16, true)
  text(36, 'data'); view.setUint32(40, samples.length * 2, true)
  for (let index = 0; index < samples.length; index += 1) {
    const clamped = Math.max(-1, Math.min(1, samples[index]))
    view.setInt16(44 + index * 2, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true)
  }
  return new Blob([bytes], { type: 'audio/wav' })
}

/// The worklet that taps the microphone, served from the app bundle. A blob URL
/// would be refused by the CSP (script-src 'self'), which is the one thing this
/// file must not rely on.
const RECORDER_WORKLET = 'voice-worklet.js'

/// Records one microphone as raw PCM and reports a 0..1 level while it runs.
///
/// Raw samples, not a MediaRecorder container: decoding a short Opus/webm blob
/// back through decodeAudioData fails ("Unable to decode audio data") whenever
/// the take is only a second or two, which is exactly the length most dictation
/// is. Tapping the graph keeps every sample the microphone produced.
export class MicRecorder {
  private stream: MediaStream | null = null
  private context: AudioContext | null = null
  private node: AudioWorkletNode | null = null
  private analyser: AnalyserNode | null = null
  private timer: number | null = null
  private chunks: Float32Array[] = []
  channels = 1
  sampleRate = 48_000

  get active() {
    return this.node !== null
  }

  async start(deviceId: string, onLevel?: (level: number) => void) {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: deviceId && deviceId !== 'default' ? { deviceId: { exact: deviceId } } : true,
    })
    this.stream = stream
    this.chunks = []
    const context = new AudioContext()
    this.context = context
    this.sampleRate = context.sampleRate
    // A context created after the click that started this may still begin
    // suspended, and a suspended graph never pulls the worklet, so no samples.
    await context.resume().catch(() => {})
    if (!context.audioWorklet) {
      this.release()
      throw new Error('This webview cannot capture raw audio, so dictation is unavailable.')
    }
    await context.audioWorklet.addModule(RECORDER_WORKLET)

    const source = context.createMediaStreamSource(stream)
    const node = new AudioWorkletNode(context, 'nova-recorder')
    node.port.onmessage = (event: MessageEvent<Float32Array>) => {
      if (event.data?.length) this.chunks.push(event.data)
    }
    // A worklet node needs a downstream connection to be pulled; a muted gain
    // keeps it running without echoing the microphone to the speakers.
    const sink = context.createGain()
    sink.gain.value = 0
    source.connect(node)
    node.connect(sink)
    sink.connect(context.destination)

    const analyser = context.createAnalyser()
    analyser.fftSize = 512
    source.connect(analyser)
    this.node = node
    this.analyser = analyser

    if (onLevel) {
      const levels = new Float32Array(analyser.fftSize)
      this.timer = window.setInterval(() => {
        analyser.getFloatTimeDomainData(levels)
        let peak = 0
        for (const value of levels) peak = Math.max(peak, Math.abs(value))
        onLevel(Math.min(1, peak * 3))
      }, 80)
    }
  }

  async stop(): Promise<Recording> {
    if (!this.node) throw new Error('Nothing is recording.')
    // Let the last render quantum land before reading the buffer.
    await new Promise((resolve) => setTimeout(resolve, 60))
    const total = this.chunks.reduce((sum, chunk) => sum + chunk.length, 0)
    const samples = new Float32Array(total)
    let offset = 0
    let peak = 0
    for (const chunk of this.chunks) {
      samples.set(chunk, offset)
      offset += chunk.length
      for (const value of chunk) peak = Math.max(peak, Math.abs(value))
    }
    const recording: Recording = {
      samples,
      sampleRate: this.sampleRate,
      channels: 1,
      duration: total / this.sampleRate,
      peak,
    }
    this.release()
    return recording
  }

  /// Drops the microphone without keeping what was recorded - used when a
  /// dictation is cancelled mid-sentence.
  cancel() {
    this.release()
  }

  private release() {
    this.node?.port.close()
    this.node?.disconnect()
    this.analyser?.disconnect()
    if (this.timer !== null) window.clearInterval(this.timer)
    this.timer = null
    this.node = null
    this.analyser = null
    this.stream?.getTracks().forEach((track) => track.stop())
    this.stream = null
    void this.context?.close()
    this.context = null
  }
}
