import { useEffect, useMemo, useState, type ReactNode } from 'react'

import { errorText } from '../lib/ipc'
import type { Preferences, SpeechProfile } from '../lib/settings'
import { SPEED_MAX, SPEED_MIN } from '../lib/settings'
import {
  PROVIDERS, cancelSpeakDownload, deleteSpeakVoice, downloadSpeakVoice, formatBytes, providerInfo,
  speakCatalog, speakInstalled, speakReply, speakRuntimeReady, stopSpeaking, voiceMeta,
  type SpeakBadge, type SpeakPhase, type SpeakProgress, type SpeakVoice,
} from '../lib/speak'

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

/// Badge text lives with the component that draws it; the catalogue only picks
/// which badges a voice carries.
const BADGE_LABELS: Record<SpeakBadge, string> = {
  recommended: 'Recommended',
  lowMemory: 'Low memory',
  multiSpeaker: 'Multi speaker',
  highestQuality: 'Highest quality',
}

const PHASE_LABELS: Record<SpeakPhase, string> = {
  runtime: 'Piper',
  voice: 'Voice',
  model: 'Model',
}

const newProfile = (): SpeechProfile => ({
  id: crypto.randomUUID(),
  name: 'Speech service',
  provider: 'openai',
  model: 'gpt-4o-mini-tts',
  voice: 'alloy',
  url: '',
  key: '',
  instructions: '',
})

export function SpeakSettings({ preferences: p, update }: Props) {
  const speech = p.speech
  const [voices, setVoices] = useState<SpeakVoice[]>([])
  const [installed, setInstalled] = useState<string[]>([])
  const [runtime, setRuntime] = useState(false)
  const [progress, setProgress] = useState<Record<string, SpeakProgress>>({})
  const [status, setStatus] = useState('')
  const [busy, setBusy] = useState(false)
  const [search, setSearch] = useState('')
  // The language is a way to narrow a catalogue of fifty voices, not a saved
  // preference: it is not sent anywhere and does not outlive the page.
  const [language, setLanguage] = useState('all')
  const [languageSearch, setLanguageSearch] = useState('')
  const [draft, setDraft] = useState<SpeechProfile | null>(null)

  const setSpeech = (patch: Partial<Preferences['speech']>) => update({ speech: { ...speech, ...patch } })

  const refresh = async () => {
    const [catalogue, present, ready] = await Promise.all([
      speakCatalog().catch(() => [] as SpeakVoice[]),
      speakInstalled().catch(() => [] as string[]),
      speakRuntimeReady().catch(() => false),
    ])
    setVoices(catalogue)
    setInstalled(present)
    setRuntime(ready)
  }

  // Not refresh(): the React Compiler lint refuses a state-setting helper
  // called straight from an effect, so the three loads stay spelled out. Each
  // call settles into its own state, so one failing arrives as an empty list
  // rather than blanking the page.
  useEffect(() => {
    void speakCatalog().then(setVoices).catch(() => setVoices([]))
    void speakInstalled().then(setInstalled).catch(() => setInstalled([]))
    void speakRuntimeReady().then(setRuntime).catch(() => setRuntime(false))
  }, [])

  // A take that ends must not keep playing while the page is closed.
  useEffect(() => () => stopSpeaking(), [])

  /// The languages the catalogue actually has, rather than a list the page
  /// invents: a voice whose language is missing would be unreachable.
  const languages = useMemo(() => {
    const seen = new Map<string, string>()
    for (const voice of voices) seen.set(voice.language.id, voice.language.name)
    return [...seen].map(([id, name]) => ({ id, name })).sort((left, right) => left.name.localeCompare(right.name))
  }, [voices])

  const shownLanguages = useMemo(() => {
    const needle = languageSearch.trim().toLowerCase()
    return [{ id: 'all', name: 'All languages' }, ...languages]
      .filter((entry) => entry.name.toLowerCase().includes(needle))
  }, [languages, languageSearch])

  const visible = useMemo(() => {
    const needle = search.trim().toLowerCase()
    const usable = voices.filter((voice) => {
      if (language !== 'all' && voice.language.id !== language) return false
      return !needle || [voice.name, voice.description, voice.licence, voice.language.name]
        .some((text) => text.toLowerCase().includes(needle))
    })
    // Installed first: the list is a control, not a shop window.
    return [...usable].sort((left, right) =>
      Number(installed.includes(right.id)) - Number(installed.includes(left.id)))
  }, [voices, installed, search, language])

  const active = voices.find((voice) => voice.id === speech.model)
  const profiles = speech.profiles
  const profile = profiles.find((entry) => entry.id === speech.profileId)

  const download = async (voice: SpeakVoice) => {
    setStatus('')
    setProgress((previous) => ({ ...previous, [voice.id]: { received: 0, total: voice.bytes, phase: 'runtime' } }))
    try {
      await downloadSpeakVoice(voice.id, (frame) => {
        setProgress((previous) => ({ ...previous, [voice.id]: frame }))
      })
      await refresh()
      setStatus(voice.name + ' is ready.')
      if (!speech.model) setSpeech({ model: voice.id })
    } catch (err) {
      setStatus(errorText(err))
    } finally {
      setProgress((previous) => {
        const next = { ...previous }
        delete next[voice.id]
        return next
      })
    }
  }

  const cancel = async (voice: SpeakVoice) => {
    try {
      await cancelSpeakDownload(voice.id)
      setStatus('Download cancelled.')
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  const remove = async (voice: SpeakVoice) => {
    try {
      await deleteSpeakVoice(voice.id)
      await refresh()
      setStatus(voice.name + ' removed.')
    } catch (err) {
      setStatus(errorText(err))
    }
  }

  const preview = async () => {
    setStatus('')
    setBusy(true)
    try {
      const error = await speakReply('Nova can read a reply aloud like this.', { ...p, speak: true })
      if (error) setStatus(error)
    } finally {
      setBusy(false)
    }
  }

  const saveProfile = (use: boolean) => {
    if (!draft) return
    const others = profiles.filter((entry) => entry.id !== draft.id)
    const saved = { ...draft, name: draft.name.trim() || 'Speech service' }
    setSpeech({ profiles: [...others, saved], profileId: use ? saved.id : speech.profileId })
    setDraft(null)
    setStatus(use ? saved.name + ' saved and selected.' : saved.name + ' saved.')
  }

  const destination = speech.destination
  // The saved destination is shown even when what it names is not installed
  // yet: "Off" over a choice the user made reads as a lost setting.
  const localReady = Boolean(active && installed.includes(active.id))
  const destinationValue = destination === 'local'
    ? 'local:' + (localReady && active ? active.id : 'missing')
    : destination === 'service'
      ? 'service:' + (profile ? profile.id : 'missing')
      : destination

  return (
    <>
      <Group title="Output">
        <label className="settings-row">
          <span>
            <strong>Read replies aloud</strong>
            <small>Reads each finished reply once. Nothing is read when a turn fails or is stopped.</small>
          </span>
          <input type="checkbox" checked={p.speak} onChange={(event) => update({ speak: event.target.checked })} />
        </label>

        <label className="settings-row">
          <span>
            <strong>Read with</strong>
            <small>
              The system voice needs no download. An on-device voice stays on this computer;
              a speech service sends the reply text to that service.
            </small>
          </span>
          <select
            className="voice-wide"
            value={destinationValue}
            onChange={(event) => {
              const next = event.target.value
              if (next === 'off') return setSpeech({ destination: 'off' })
              if (next === 'system') return setSpeech({ destination: 'system' })
              if (next.startsWith('local:')) return setSpeech({ destination: 'local', model: next.slice('local:'.length) })
              if (next.startsWith('service:')) return setSpeech({ destination: 'service', profileId: next.slice('service:'.length) })
            }}
          >
            <option value="off">Off</option>
            <option value="system">System voice</option>
            {destination === 'local' && !localReady && (
              <option value="local:missing" disabled>On-device (not installed)</option>
            )}
            {destination === 'service' && !profile && (
              <option value="service:missing" disabled>Speech service (none saved)</option>
            )}
            {installed.length === 0 && <option value="local:none" disabled>On-device (no voice installed)</option>}
            {voices.filter((voice) => installed.includes(voice.id)).map((voice) => (
              <option key={voice.id} value={'local:' + voice.id}>{'On-device (' + voice.name + ')'}</option>
            ))}
            {profiles.length === 0 && <option value="service:none" disabled>Speech service (none saved)</option>}
            {profiles.map((entry) => (
              <option key={entry.id} value={'service:' + entry.id}>{'Service (' + entry.name + ')'}</option>
            ))}
          </select>
        </label>

        <label className="settings-row">
          <span>
            <strong>Speed</strong>
            <small>{'How fast it reads. 1 is the voice\'s own pace; the engine accepts ' + SPEED_MIN + ' to ' + SPEED_MAX + '.'}</small>
          </span>
          <input
            className="speech-speed"
            type="range"
            min={SPEED_MIN}
            max={SPEED_MAX}
            step={0.1}
            value={speech.speed}
            onChange={(event) => setSpeech({ speed: Number(event.target.value) })}
          />
        </label>

        {active && active.speakers > 1 && installed.includes(active.id) && (
          <label className="settings-row">
            <span>
              <strong>Speaker</strong>
              <small>{active.name + ' carries ' + active.speakers + ' readers; this one is used for every reply.'}</small>
            </span>
            <input
              className="speech-speaker"
              type="number"
              min={0}
              max={active.speakers - 1}
              value={speech.speaker}
              onChange={(event) => setSpeech({ speaker: Math.max(0, Math.min(active.speakers - 1, Number(event.target.value) || 0)) })}
            />
          </label>
        )}

        <div className="settings-row">
          <span>
            <strong>Preview</strong>
            <small>Reads one short sentence with the settings above, here and now.</small>
          </span>
          <div className="voice-row-controls">
            <button type="button" onClick={() => void preview()} disabled={busy}>{busy ? 'Reading…' : 'Preview voice'}</button>
            <button type="button" onClick={() => { stopSpeaking(); setStatus('') }}>Stop</button>
          </div>
        </div>
      </Group>

      <Group title="Read-aloud services">
        <p className="settings-help">
          Send the reply text to a speech service instead of a voice on this computer.
          Keys are kept in this window's local storage, the same place the model keys live,
          and saving never contacts the service.
        </p>
        <label className="settings-row">
          <span>
            <strong>Saved services</strong>
            <small>{profiles.length === 0 ? 'No speech service saved yet.' : profiles.length === 1 ? '1 saved.' : profiles.length + ' saved.'}</small>
          </span>
          <div className="voice-row-controls">
            <select
              className="voice-wide"
              value={speech.profileId}
              onChange={(event) => {
                const id = event.target.value
                setSpeech({ profileId: id, destination: id ? 'service' : speech.destination === 'service' ? 'system' : speech.destination })
              }}
            >
              <option value="">None</option>
              {profiles.map((entry) => (
                <option key={entry.id} value={entry.id}>{entry.name + ' · ' + providerInfo(entry.provider).name}</option>
              ))}
            </select>
            <button type="button" onClick={() => setDraft(newProfile())}>New profile</button>
          </div>
        </label>

        {profiles.length > 0 && (
          <div className="speech-profiles">
            {profiles.map((entry) => (
              <article className="voice-card" key={entry.id} data-installed={entry.id === speech.profileId}>
                <header>
                  <strong>{entry.name}</strong>
                  <em className="voice-badge">{providerInfo(entry.provider).name}</em>
                </header>
                <p>{providerInfo(entry.provider).note}</p>
                <small className="voice-meta">{entry.model || 'no model'} · {entry.voice || 'no voice'}{entry.key ? '' : ' · no key'}</small>
                <div className="voice-actions">
                  <button type="button" onClick={() => setDraft({ ...entry })}>Edit</button>
                  <button type="button" onClick={() => {
                    const kept = profiles.filter((other) => other.id !== entry.id)
                    setSpeech({
                      profiles: kept,
                      profileId: speech.profileId === entry.id ? (kept[0]?.id ?? '') : speech.profileId,
                      destination: speech.profileId === entry.id && speech.destination === 'service' ? 'system' : speech.destination,
                    })
                  }}>Delete</button>
                </div>
              </article>
            ))}
          </div>
        )}

        {draft && (
          <section className="speech-profile">
            <h3>{profiles.some((entry) => entry.id === draft.id) ? 'Edit speech profile' : 'New speech profile'}</h3>
            <label className="settings-row">
              <span><strong>Service</strong><small>{providerInfo(draft.provider).note}</small></span>
              <select
                value={draft.provider}
                onChange={(event) => {
                  const provider = event.target.value as SpeechProfile['provider']
                  const info = providerInfo(provider)
                  setDraft({
                    ...draft,
                    provider,
                    model: info.models[0] ?? '',
                    voice: info.voices[0] ?? '',
                    instructions: info.instructions ? draft.instructions : '',
                  })
                }}
              >
                {PROVIDERS.map((provider) => (
                  <option key={provider.id} value={provider.id}>{provider.name}</option>
                ))}
              </select>
            </label>

            <label className="settings-row">
              <span><strong>Name</strong><small>What this profile is called in the list.</small></span>
              <input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
            </label>

            <label className="settings-row">
              <span><strong>Model</strong><small>{providerInfo(draft.provider).models.length ? 'The models this service publishes.' : 'Whatever your server expects.'}</small></span>
              {providerInfo(draft.provider).models.length > 0 ? (
                <select value={draft.model} onChange={(event) => setDraft({ ...draft, model: event.target.value })}>
                  {providerInfo(draft.provider).models.map((model) => (
                    <option key={model} value={model}>{model}</option>
                  ))}
                </select>
              ) : (
                <input value={draft.model} placeholder="model id" onChange={(event) => setDraft({ ...draft, model: event.target.value })} />
              )}
            </label>

            <label className="settings-row">
              <span><strong>Voice</strong><small>{draft.provider === 'elevenlabs' ? 'A voice id from your account.' : 'The voice name the service expects.'}</small></span>
              {providerInfo(draft.provider).voices.length > 0 ? (
                <select value={draft.voice} onChange={(event) => setDraft({ ...draft, voice: event.target.value })}>
                  {providerInfo(draft.provider).voices.map((voice) => (
                    <option key={voice} value={voice}>{voice}</option>
                  ))}
                </select>
              ) : (
                <input value={draft.voice} placeholder="voice id" onChange={(event) => setDraft({ ...draft, voice: event.target.value })} />
              )}
            </label>

            {draft.provider === 'custom' && (
              <label className="settings-row">
                <span><strong>Address</strong><small>An https URL that speaks OpenAI's /v1/audio/speech shape.</small></span>
                <input value={draft.url} placeholder="https://speech.example.com/v1/audio/speech"
                  onChange={(event) => setDraft({ ...draft, url: event.target.value })} />
              </label>
            )}

            <label className="settings-row">
              <span><strong>API key</strong><small>Kept in this window's local storage and sent only to this service.</small></span>
              <input type="password" value={draft.key} autoComplete="off"
                onChange={(event) => setDraft({ ...draft, key: event.target.value })} />
            </label>

            {providerInfo(draft.provider).instructions && (
              <label className="settings-row">
                <span><strong>Style instruction</strong><small>Optional, and only OpenAI's speech API has a field for it.</small></span>
                <input value={draft.instructions} placeholder="Read it like a tired engineer."
                  onChange={(event) => setDraft({ ...draft, instructions: event.target.value })} />
              </label>
            )}

            <div className="speech-actions">
              <button type="button" onClick={() => setDraft(null)}>Cancel</button>
              <button type="button" onClick={() => saveProfile(false)}>Save profile</button>
              <button type="button" onClick={() => saveProfile(true)}>Save and use</button>
            </div>
          </section>
        )}
      </Group>

      <Group title="On-device voices">
        <p className="settings-help">
          A voice reads on this computer. Most of them go through Piper: the first voice also
          installs Piper itself (about 22 MB), then the voice (about 63 MB). Kale runs in this app
          instead, on weights it brings with it, so it needs no install beside it and no Piper.
          Voices are MIT-licensed; the binary Piper downloads carries espeak-ng, which is GPLv3 and
          stays outside Nova's own executable.
        </p>
        <p className="settings-help" role="status">
          {runtime ? 'Piper is installed.' : 'Piper is not installed yet; it arrives with the first voice.'}
        </p>
        <div className="voice-panes">
          <div className="voice-pane voice-languages">
            <h3>Languages</h3>
            <input className="voice-search" aria-label="Search languages" placeholder="Search languages"
              value={languageSearch} onChange={(event) => setLanguageSearch(event.target.value)} />
            <ul>
              {shownLanguages.map((entry) => (
                <li key={entry.id}>
                  <button type="button" aria-pressed={language === entry.id}
                    onClick={() => setLanguage(entry.id)}>
                    {entry.name}{language === entry.id && <span aria-hidden="true">✓</span>}
                  </button>
                </li>
              ))}
            </ul>
          </div>
          <div className="voice-pane voice-models">
            <h3>{language === 'all' ? 'Voices for all languages'
              : 'Voices for ' + (languages.find((entry) => entry.id === language)?.name ?? language)}</h3>
            <input className="voice-search" aria-label="Search voices" placeholder="Search voices"
              value={search} onChange={(event) => setSearch(event.target.value)} />
            <div className="voice-cards">
          {visible.map((voice) => {
            const running = progress[voice.id]
            // Piper is shared by every Piper voice, so an installed voice is
            // only usable once the runtime is there too. An in-process voice
            // brings its own everything and has nothing to wait for.
            const present = installed.includes(voice.id) && (runtime || voice.engine !== 'piper')
            return (
              <article className="voice-card" key={voice.id} data-installed={present}>
                <header>
                  <strong>{voice.name}</strong>
                  {voice.badges.map((badge) => <em key={badge} className="voice-badge">{BADGE_LABELS[badge]}</em>)}
                </header>
                <p>{voice.description}</p>
                <small className="voice-meta">{voiceMeta(voice)}</small>
                <small className="voice-licence">{voice.licence}</small>
                <div className="voice-actions">
                  {!voice.supported && (
                    <button type="button" disabled title={voice.unsupportedReason ?? undefined}>Not supported here</button>
                  )}
                  {voice.supported && running && (
                    <>
                      <div className="voice-progress" role="progressbar" aria-valuemin={0}
                        aria-valuemax={running.total} aria-valuenow={running.received}
                        aria-label={'Downloading ' + PHASE_LABELS[running.phase]}>
                        <i style={{ width: (running.total ? Math.round((running.received / running.total) * 100) : 0) + '%' }} />
                      </div>
                      <small>{PHASE_LABELS[running.phase] + ': ' + formatBytes(running.received) + ' of ' + formatBytes(running.total)}</small>
                      <button type="button" onClick={() => void cancel(voice)}>Cancel</button>
                    </>
                  )}
                  {voice.supported && !running && !present && (
                    <button type="button" onClick={() => void download(voice)}
                      title={'Download ' + voice.name + ' (' + formatBytes(voice.bytes) + ')'}>Download</button>
                  )}
                  {voice.supported && !running && present && (
                    <>
                      <span className="voice-installed">{'Installed · ' + formatBytes(voice.bytes)}</span>
                      <button type="button" onClick={() => void remove(voice)}>Delete</button>
                    </>
                  )}
                </div>
              </article>
            )
          })}
              {visible.length === 0 && (
                <p className="settings-help">{voices.length === 0
                  ? 'The catalogue has not loaded, or this build ships no voices.'
                  : 'No voice matches that filter.'}</p>
              )}
            </div>
          </div>
        </div>
      </Group>

      {status && <p className="settings-help" role="status">{status}</p>}
    </>
  )
}

export default SpeakSettings
