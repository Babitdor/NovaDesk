import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'

import { errorText } from '../lib/ipc'
import type { Preferences } from '../lib/settings'
import {
  VOICE_SHORTCUT_LABEL, MicRecorder, cancelVoiceDownload, catalogueLanguages, deleteVoiceModel,
  downloadVoiceModel, formatBytes, modelMeta, toWav, voiceCatalog, voiceEngineReady, voiceInstalled,
  type VoiceBadge, type VoiceModel,
} from '../lib/voice'

type Props = {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
}

type Download = { received: number; total: number }

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="settings-group">
      {title && <h2>{title}</h2>}
      {children}
    </section>
  )
}

const deviceLabel = (device: MediaDeviceInfo, index: number) =>
  device.label || 'Microphone ' + (index + 1)

/// Badge text lives with the component that draws it; the catalogue only picks
/// which badges a model carries.
const BADGE_LABELS: Record<VoiceBadge, string> = {
  recommended: 'Recommended',
  translate: 'Translate',
  livePreview: 'Live preview',
  streaming: 'Streaming',
}

export function VoiceSettings({ preferences: p, update }: Props) {
  const voice = p.voice
  const [models, setModels] = useState<VoiceModel[]>([])
  const [installed, setInstalled] = useState<string[]>([])
  const [progress, setProgress] = useState<Record<string, Download>>({})
  const [status, setStatus] = useState('')
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const [language, setLanguage] = useState(voice.language)
  const [languageSearch, setLanguageSearch] = useState('')
  const [modelSearch, setModelSearch] = useState('')
  const [engineReady, setEngineReady] = useState(true)
  const [testing, setTesting] = useState(false)
  const [level, setLevel] = useState(0)
  const [clip, setClip] = useState('')
  const [testError, setTestError] = useState('')
  const recorder = useRef<MicRecorder | null>(null)
  const stopTimer = useRef<number | null>(null)

  const setVoice = (patch: Partial<Preferences['voice']>) => update({ voice: { ...voice, ...patch } })

  const refreshModels = async () => {
    try {
      const [catalogue, present] = await Promise.all([voiceCatalog(), voiceInstalled()])
      setModels(catalogue)
      setInstalled(present)
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  const refreshDevices = async () => {
    try {
      const all = await navigator.mediaDevices.enumerateDevices()
      setDevices(all.filter((device) => device.kind === 'audioinput'))
    } catch {
      setDevices([])
    }
  }

  // First load: the catalogue, what is installed, the microphones and whether
  // the runtime starts. Each settles into its own state, so one failing call
  // does not blank the page.
  useEffect(() => {
    void voiceCatalog().then(setModels).catch((err) => setStatus(errorText(err)))
    void voiceInstalled().then(setInstalled).catch(() => setInstalled([]))
    void voiceEngineReady().then(setEngineReady).catch(() => setEngineReady(false))
    void navigator.mediaDevices
      .enumerateDevices()
      .then((all) => setDevices(all.filter((device) => device.kind === 'audioinput')))
      .catch(() => setDevices([]))
  }, [])

  useEffect(() => () => {
    if (clip) URL.revokeObjectURL(clip)
  }, [clip])

  useEffect(() => () => {
    if (stopTimer.current !== null) window.clearTimeout(stopTimer.current)
    recorder.current?.cancel()
  }, [])

  const languages = useMemo(() => catalogueLanguages(models), [models])
  const visible = useMemo(() => {
    const needle = modelSearch.trim().toLowerCase()
    return models.filter((model) => {
      if (language !== 'all' && !model.languages.some((entry) => entry.id === language)) return false
      if (!needle) return true
      return [model.name, model.description, model.licence, ...model.languages.map((entry) => entry.name)]
        .some((text) => text.toLowerCase().includes(needle))
    })
  }, [models, language, modelSearch])

  const usable = models.find((model) => model.id === voice.model && installed.includes(model.id))

  const startDownload = async (model: VoiceModel) => {
    setStatus('')
    setProgress((previous) => ({ ...previous, [model.id]: { received: 0, total: model.bytes } }))
    try {
      await downloadVoiceModel(model.id, (frame) => {
        setProgress((previous) => ({ ...previous, [model.id]: frame }))
      })
      await refreshModels()
      setStatus(model.name + ' is ready.')
      if (!voice.model) setVoice({ model: model.id })
    } catch (err) {
      setStatus(errorText(err))
    } finally {
      setProgress((previous) => {
        const next = { ...previous }
        delete next[model.id]
        return next
      })
    }
  }

  const cancel = async (model: VoiceModel) => {
    try {
      await cancelVoiceDownload(model.id)
      setStatus('Download cancelled.')
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  const remove = async (model: VoiceModel) => {
    try {
      await deleteVoiceModel(model.id)
      await refreshModels()
      setStatus(model.name + ' removed.')
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  const startTest = async () => {
    setTestError('')
    setClip((previous) => {
      if (previous) URL.revokeObjectURL(previous)
      return ''
    })
    try {
      const mic = (recorder.current ??= new MicRecorder())
      await mic.start(voice.microphone, setLevel)
      setTesting(true)
      // Test audio stays on this device: ten seconds, then stop by itself.
      stopTimer.current = window.setTimeout(() => void stopTest(), 10_000)
      void refreshDevices()
    } catch (err) {
      setTesting(false)
      setTestError(err instanceof Error ? err.message : String(err))
    }
  }

  const stopTest = async () => {
    const mic = recorder.current
    if (stopTimer.current !== null) window.clearTimeout(stopTimer.current)
    stopTimer.current = null
    setTesting(false)
    setLevel(0)
    if (!mic?.active) return
    try {
      const recording = await mic.stop()
      if (!recording.samples.length) {
        setTestError('The microphone sent no audio. Check the input device and try again.')
        return
      }
      setClip(URL.createObjectURL(toWav(recording)))
    } catch (err) {
      setTestError(err instanceof Error ? err.message : String(err))
    }
  }

  const destination = voice.destination === 'local' && usable ? 'local' : 'off'

  return (
    <>
      <Group title="Input">
        <div className="settings-row">
          <span>
            <strong>Microphone</strong>
            <small>System default follows the OS. Test microphone records a 10-second test and plays it back here. Test audio stays on this device and is never transcribed.</small>
          </span>
          <div className="voice-row-controls">
            <button type="button" onClick={() => void (testing ? stopTest() : startTest())}>
              {testing ? 'Stop recording' : 'Test microphone'}
            </button>
            <button type="button" className="voice-icon" aria-label="Refresh microphones"
              title="Refresh the microphone list" onClick={() => void refreshDevices()}>↻</button>
          </div>
        </div>
        <label className="settings-row">
          <span><strong>Audio device</strong><small>{devices.length === 0 ? 'Allow microphone access to see device names.'
            : devices.length === 1 ? '1 input found.' : devices.length + ' inputs found.'}</small></span>
          <select value={voice.microphone} onChange={(event) => setVoice({ microphone: event.target.value })}>
            <option value="default">System default</option>
            {devices.map((device, index) => (
              <option key={device.deviceId || String(index)} value={device.deviceId}>
                {deviceLabel(device, index)}
              </option>
            ))}
          </select>
        </label>
        {(testing || level > 0) && <div className="voice-level" aria-hidden="true"><i style={{ width: Math.round(level * 100) + '%' }} /></div>}
        {testError && <p className="settings-help voice-error" role="status">{testError}</p>}
        {clip && <audio className="voice-clip" controls src={clip} aria-label="Test recording" />}
        <label className="settings-row">
          <span><strong>Dictation shortcut</strong><small>{'How ' + VOICE_SHORTCUT_LABEL + ' starts and stops recording.'}</small></span>
          <select value={voice.shortcut} onChange={(event) => setVoice({ shortcut: event.target.value as Preferences['voice']['shortcut'] })}>
            <option value="toggle">Press to toggle</option>
            <option value="hold">Hold to record</option>
          </select>
        </label>
      </Group>

      <Group title="Transcription">
        <label className="settings-row">
          <span>
            <strong>Send microphone audio to</strong>
            <small>Audio is transcribed here and discarded; nothing is uploaded in this build. Choose Off to turn voice input off on this device.</small>
          </span>
          <select className="voice-wide" value={destination === 'local' && usable ? 'local:' + usable.id : 'off'}
            onChange={(event) => {
              const next = event.target.value
              if (next === 'off') { setVoice({ destination: 'off' }); return }
              const id = next.slice('local:'.length)
              if (!installed.includes(id)) return
              setVoice({ destination: 'local', model: id })
            }}>
            <option value="off">Off</option>
            {installed.length === 0 && <option value="local:none" disabled>On-device (no model installed)</option>}
            {models.filter((model) => installed.includes(model.id)).map((model) => (
              <option key={model.id} value={'local:' + model.id}>{'On-device (' + model.name + ')'}</option>
            ))}
          </select>
        </label>
        {!engineReady && <p className="settings-help voice-error" role="status">
          On-device transcription is unavailable: the ONNX Runtime environment did not start in this build, so every transcription would fail.
        </p>}
        <p className="settings-help">
          Pick languages to see models that support all of them. Accuracy is word or character error rate: lower is better.
          Speed is the time to transcribe 30 seconds of audio, measured on the Handy project's benchmark GPU.
          Grades and speeds come from the Handy models benchmark.
        </p>
        <div className="voice-panes">
          <div className="voice-pane voice-languages">
            <h3>Languages</h3>
            <input className="voice-search" aria-label="Search languages" placeholder="Search languages" value={languageSearch}
              onChange={(event) => setLanguageSearch(event.target.value)} />
            <ul>
              {[{ id: 'all', name: 'All languages' }, ...languages]
                .filter((entry) => entry.name.toLowerCase().includes(languageSearch.trim().toLowerCase()))
                .map((entry) => (
                  <li key={entry.id}>
                    <button type="button" aria-pressed={language === entry.id}
                      onClick={() => { setLanguage(entry.id); setVoice({ language: entry.id }) }}>
                      {entry.name}{language === entry.id && <span aria-hidden="true">✓</span>}
                    </button>
                  </li>
                ))}
            </ul>
          </div>
          <div className="voice-pane voice-models">
            <h3>{language === 'all' ? 'Models for all languages' : 'Models for ' + (languages.find((entry) => entry.id === language)?.name ?? language)}</h3>
            <input className="voice-search" aria-label="Search models" placeholder="Search models" value={modelSearch}
              onChange={(event) => setModelSearch(event.target.value)} />
            <div className="voice-cards">
              {visible.map((model) => {
                const running = progress[model.id]
                const present = installed.includes(model.id)
                return (
                  <article className="voice-card" key={model.id} data-installed={present}>
                    <header>
                      <strong>{model.name}</strong>
                      {model.badges.map((badge) => <em key={badge} className="voice-badge">{BADGE_LABELS[badge]}</em>)}
                    </header>
                    <p>{model.description}</p>
                    <small className="voice-meta">{modelMeta(model)}</small>
                    <small className="voice-licence">{model.licence}</small>
                    <div className="voice-actions">
                      {!model.supported && (
                        <button type="button" disabled title={model.unsupportedReason ?? undefined}>Not supported yet</button>
                      )}
                      {model.supported && running && (
                        <>
                          <div className="voice-progress" role="progressbar" aria-valuemin={0}
                            aria-valuemax={running.total} aria-valuenow={running.received}
                            aria-label={'Downloading ' + model.name}>
                            <i style={{ width: (running.total ? Math.round((running.received / running.total) * 100) : 0) + '%' }} />
                          </div>
                          <small>{formatBytes(running.received) + ' of ' + formatBytes(running.total)}</small>
                          <button type="button" onClick={() => void cancel(model)}>Cancel</button>
                        </>
                      )}
                      {model.supported && !running && !present && (
                        <button type="button" onClick={() => void startDownload(model)} title={'Download ' + model.name + ' (' + formatBytes(model.bytes) + ')'}>Download</button>
                      )}
                      {model.supported && !running && present && (
                        <>
                          <span className="voice-installed">{'Installed · ' + formatBytes(model.bytes)}</span>
                          <button type="button" onClick={() => void remove(model)}>Delete</button>
                        </>
                      )}
                    </div>
                  </article>
                )
              })}
              {visible.length === 0 && <p className="settings-help">No model matches that search.</p>}
            </div>
          </div>
        </div>
        {status && <p className="settings-help" role="status">{status}</p>}
      </Group>
    </>
  )
}

export default VoiceSettings
