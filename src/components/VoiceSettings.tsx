import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'

import { errorText } from '../lib/ipc'
import type { Preferences, TranscriptionProfile, TranscriptionProvider } from '../lib/settings'
import {
  VOICE_SHORTCUT_LABEL, MicRecorder, cancelVoiceDownload, catalogueLanguages, deleteVoiceModel,
  downloadVoice, formatBytes, modelMeta, toWav, transcribeProviders, voiceCatalog, voiceDownloads,
  voiceEngineReady, voiceInstalled, type TranscriptionProviderInfo, type VoiceBadge, type VoiceModel,
} from '../lib/voice'

type Props = {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
}

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
  // Read from the module, not from state: the transfer outlives this pane, so
  // the row has to remember it across a section switch.
  const progress = useSyncExternalStore(voiceDownloads.subscribe, voiceDownloads.snapshot)
  const [status, setStatus] = useState('')
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const [language, setLanguage] = useState(voice.language)
  const [languageSearch, setLanguageSearch] = useState('')
  const [modelSearch, setModelSearch] = useState('')
  const [engineReady, setEngineReady] = useState(true)
  const [providers, setProviders] = useState<TranscriptionProviderInfo[]>([])
  const [draft, setDraft] = useState<TranscriptionProfile | null>(null)
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
    void transcribeProviders().then(setProviders).catch(() => setProviders([]))
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
    try {
      await downloadVoice(model.id, model.bytes)
      await refreshModels()
      setStatus(model.name + ' is ready.')
      if (!voice.model) setVoice({ model: model.id })
    } catch (err) {
      setStatus(errorText(err))
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

  const profileInfo = (provider: TranscriptionProvider) =>
    providers.find((entry) => entry.id === provider)

  const profileName = (provider: TranscriptionProvider) =>
    profileInfo(provider)?.name ?? provider

  const active = voice.profiles.find((entry) => entry.id === voice.profileId)

  // What the destination select shows. A saved service that was deleted, or a
  // model that is no longer installed, is not a destination any more.
  const destination = voice.destination === 'service' && active
    ? 'service:' + active.id
    : voice.destination === 'local' && usable
      ? 'local:' + usable.id
      : 'off'

  const newProfile = (): TranscriptionProfile => {
    const info = providers[0]
    return {
      id: crypto.randomUUID(),
      name: info?.name ?? 'Speech service',
      provider: info?.id ?? 'openai',
      model: info?.models[0] ?? '',
      url: '',
      key: '',
    }
  }

  // Save keeps the profile in the list; Save and use also points dictation at
  // it, which is the only way the destination changes from one button.
  const saveProfile = (use: boolean) => {
    if (!draft) return
    const entry = { ...draft, name: draft.name.trim() || profileName(draft.provider) }
    const exists = voice.profiles.some((other) => other.id === entry.id)
    const profiles = exists
      ? voice.profiles.map((other) => (other.id === entry.id ? entry : other))
      : [...voice.profiles, entry]
    setVoice(use
      ? { profiles, profileId: entry.id, destination: 'service' }
      : { profiles })
    setDraft(null)
    setStatus(use ? entry.name + ' saved and in use.' : entry.name + ' saved.')
  }

  const removeProfile = (entry: TranscriptionProfile) => {
    const kept = voice.profiles.filter((other) => other.id !== entry.id)
    setVoice({
      profiles: kept,
      ...(voice.profileId === entry.id
        ? { profileId: kept[0]?.id ?? '', destination: voice.destination === 'service' ? 'off' : voice.destination }
        : {}),
    })
    setStatus(entry.name + ' removed.')
  }

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

      <Group title="Speech services">
        <p className="settings-help">
          Send the recording to a service such as OpenAI, Groq, Mistral or your own server instead of a
          model on this computer. Keys are kept in this window's local storage, the same place the model
          keys live, and saving never contacts the service.
        </p>
        <div className="settings-row">
          <span>
            <strong>Saved services</strong>
            <small>{voice.profiles.length === 0 ? 'No speech profile yet. Add one to enable dictation.'
              : voice.profiles.length === 1 ? '1 saved.' : voice.profiles.length + ' saved.'}</small>
          </span>
          <div className="voice-row-controls">
            <button type="button" onClick={() => setDraft(newProfile())}>New profile</button>
          </div>
        </div>
        {voice.profiles.length > 0 && (
          <div className="speech-profiles">
            {voice.profiles.map((entry) => (
              <article className="voice-card" key={entry.id} data-installed={entry.id === voice.profileId}>
                <header>
                  <strong>{entry.name}</strong>
                  <em className="voice-badge">{profileName(entry.provider)}</em>
                </header>
                <p>{profileInfo(entry.provider)?.note ?? 'A saved transcription service.'}</p>
                <small className="voice-meta">{entry.model || 'no model'}{entry.key ? '' : ' · no key'}</small>
                <div className="voice-actions">
                  <button type="button" onClick={() => setDraft({ ...entry })}>Edit</button>
                  <button type="button" onClick={() => removeProfile(entry)}>Delete</button>
                  {entry.id !== voice.profileId && (
                    <button type="button" onClick={() => setVoice({ profileId: entry.id, destination: 'service' })}>Use</button>
                  )}
                </div>
              </article>
            ))}
          </div>
        )}
        {draft && (
          <section className="speech-profile">
            <h3>{voice.profiles.some((entry) => entry.id === draft.id) ? 'Edit speech profile' : 'New speech profile'}</h3>
            <label className="settings-row">
              <span>
                <strong>Speech provider</strong>
                <small>{profileInfo(draft.provider)?.note ?? 'Where the recording is sent.'}</small>
              </span>
              <select value={draft.provider}
                onChange={(event) => {
                  const provider = event.target.value as TranscriptionProvider
                  const info = profileInfo(provider)
                  setDraft({ ...draft, provider, model: info?.models[0] ?? '', url: provider === 'custom' ? draft.url : '' })
                }}>
                {providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>{provider.name}</option>
                ))}
              </select>
            </label>
            <label className="settings-row">
              <span>
                <strong>Speech model</strong>
                <small>{(profileInfo(draft.provider)?.models.length ?? 0) > 0
                  ? 'The models this service publishes for transcription.'
                  : 'Whatever your server expects.'}</small>
              </span>
              {(profileInfo(draft.provider)?.models.length ?? 0) > 0 ? (
                <select value={draft.model} onChange={(event) => setDraft({ ...draft, model: event.target.value })}>
                  {profileInfo(draft.provider)?.models.map((model) => (
                    <option key={model} value={model}>{model}</option>
                  ))}
                </select>
              ) : (
                <input value={draft.model} placeholder="model id"
                  onChange={(event) => setDraft({ ...draft, model: event.target.value })} />
              )}
            </label>
            <label className="settings-row">
              <span><strong>Profile name</strong><small>What this profile is called in the list.</small></span>
              <input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
            </label>
            <label className="settings-row">
              <span>
                <strong>API key</strong>
                <small>Enter an API key from this speech provider. It is stored in this window's local storage.</small>
              </span>
              <input type="password" value={draft.key} placeholder="paste the key"
                onChange={(event) => setDraft({ ...draft, key: event.target.value })} />
            </label>
            <details className="speech-advanced">
              <summary>Advanced</summary>
              <div className="settings-row">
                <span>
                  <strong>Destination</strong>
                  <small>Saving does not contact the service. The recording is posted only when you dictate.</small>
                </span>
                {draft.provider === 'custom' ? (
                  <input value={draft.url} placeholder="https://your-server/v1/audio/transcriptions"
                    onChange={(event) => setDraft({ ...draft, url: event.target.value })} />
                ) : (
                  <code className="speech-destination">{profileInfo(draft.provider)?.url ?? ''}</code>
                )}
              </div>
            </details>
            <div className="voice-row-controls speech-actions">
              <button type="button" onClick={() => setDraft(null)}>Cancel</button>
              <button type="button" onClick={() => saveProfile(false)}>Save profile</button>
              <button type="button" onClick={() => saveProfile(true)}>Save and use profile</button>
            </div>
          </section>
        )}
      </Group>

      <Group title="Transcription">
        <label className="settings-row">
          <span>
            <strong>Send microphone audio to</strong>
            <small>On-device models read the audio here and discard it. A saved service is sent the recording instead, and nothing else leaves this computer. Choose Off to turn voice input off.</small>
          </span>
          <select className="voice-wide" value={destination}
            onChange={(event) => {
              const next = event.target.value
              if (next === 'off') { setVoice({ destination: 'off' }); return }
              if (next.startsWith('service:')) {
                const id = next.slice('service:'.length)
                if (!voice.profiles.some((entry) => entry.id === id)) return
                setVoice({ destination: 'service', profileId: id })
                return
              }
              const id = next.slice('local:'.length)
              if (!installed.includes(id)) return
              setVoice({ destination: 'local', model: id })
            }}>
            <option value="off">Off</option>
            {installed.length === 0 && <option value="local:none" disabled>On-device (no model installed)</option>}
            {models.filter((model) => installed.includes(model.id)).map((model) => (
              <option key={model.id} value={'local:' + model.id}>{'On-device (' + model.name + ')'}</option>
            ))}
            {voice.profiles.map((entry) => (
              <option key={entry.id} value={'service:' + entry.id}>{profileName(entry.provider) + ' (' + entry.name + ')'}</option>
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
                const running = progress.get(model.id)
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
