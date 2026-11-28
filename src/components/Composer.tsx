import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react'

import type { SlashCommand } from '../lib/slash'
import { AGENTS, type AgentId, type ModelSelection, type Preferences } from '../lib/settings'
import { AgentMark } from './AgentMark'
import { ModelPicker } from './ModelPicker'
import { CREW, addressedCrew, crewRoles, crewTargets, type CrewMode } from '../lib/crew'
import { CrewPortrait } from './CrewPortrait'
import { VOICE_SHORTCUT_LABEL, MicRecorder, voiceInstalled, voiceTranscribe, voiceTranscribeService } from '../lib/voice'
import { errorText, pickAttachments } from '../lib/ipc'
import { loadDrafts, saveDraft } from '../lib/drafts'
import { attachmentPrompt } from '../lib/attachments'

type Props = {
  chatId: string
  commands: SlashCommand[]
  busy: boolean
  disabled: boolean
  agent: AgentId
  model?: string
  effort?: string
  recentModels: ModelSelection[]
  configuredModels: Preferences['models']
  onSelectModel: (selection: ModelSelection) => void
  hasHistory: boolean
  work: string
  waiting: boolean
  obscured: boolean
  crew?: CrewMode
  onCrew: (mode?: CrewMode) => void
  onConfigureCrew: () => void
  usage?: { input: number; output: number }
  workspace: string | null
  voice: Preferences['voice']
  onOpenVoice: () => void
  /// "Keep unsent drafts": the store is only touched while the preference is on.
  keepDrafts: boolean
  autoApprove: boolean
  onAutoApprove: (enabled: boolean) => void
  onSend: (prompt: string) => boolean
  onStop: () => void
}

/// The composer's own drawn icons, at the weight of the labels beside them and
/// aria-hidden: each sits inside a control that is already named.
function Glyph({ name }: { name: 'attach' | 'lock' | 'lock-open' | 'people' }) {
  const shape = () => {
    switch (name) {
      case 'attach':
        return <path d="M11.5 7.5 7 12a2.8 2.8 0 0 1-4-4l5-5a1.9 1.9 0 0 1 2.7 2.7l-5 5a1 1 0 0 1-1.4-1.4l4.4-4.4"
          fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
      case 'lock':
        return (
          <>
            <rect x="3.5" y="7" width="9" height="6.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
            <path d="M5.75 7V5.2a2.25 2.25 0 0 1 4.5 0V7" fill="none" stroke="currentColor" strokeWidth="1.4" />
          </>
        )
      case 'lock-open':
        return (
          <>
            <rect x="3.5" y="7" width="9" height="6.5" fill="none" stroke="currentColor" strokeWidth="1.4" />
            <path d="M5.75 7V5.2a2.25 2.25 0 0 1 4.5 0" fill="none" stroke="currentColor" strokeWidth="1.4" />
          </>
        )
      case 'people':
        return (
          <>
            <circle cx="6" cy="6" r="2.2" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <path d="M2 13c0-2.2 1.8-3.5 4-3.5s4 1.3 4 3.5" fill="none" stroke="currentColor" strokeWidth="1.3" />
            <path d="M10.5 4.2a2 2 0 0 1 0 3.8M11.8 9.8c1.4.4 2.2 1.5 2.2 3.2"
              fill="none" stroke="currentColor" strokeWidth="1.3" />
          </>
        )
    }
  }

  return (
    <svg width="14" height="14" viewBox="0 0 16 16" aria-hidden="true">
      {shape()}
    </svg>
  )
}

export function Composer({
  chatId, commands, busy, disabled, agent, model, effort, recentModels, configuredModels, onSelectModel,
  hasHistory, work, waiting, obscured, crew, onCrew, onConfigureCrew, usage, workspace, voice,
  onOpenVoice, keepDrafts, autoApprove, onAutoApprove, onSend, onStop,
}: Props) {
  const composer = useRef<HTMLDivElement>(null)
  const modelButton = useRef<HTMLButtonElement>(null)
  const input = useRef<HTMLTextAreaElement>(null)
  const menuId = useId()
  const modelId = useId()
  const [drafts, setDrafts] = useState<Record<string, string>>(() => keepDrafts ? loadDrafts() : {})
  const [menu, setMenu] = useState<{ chat: string; kind: 'slash' | 'mention'; highlighted: number } | null>(null)
  const [modelMenu, setModelMenu] = useState<string | null>(null)
  if (modelMenu && (modelMenu !== chatId || obscured || busy)) setModelMenu(null)
  const modelOpen = modelMenu === chatId
  const backend = AGENTS[agent]?.name ?? agent
  const recipient = crew === 'team' ? 'the team' : crew ? CREW[crew].name : backend
  const draft = drafts[chatId] ?? ''
  const slash = /^\/([a-z0-9_?:-]*)$/i.exec(draft.trimStart())
  const mention = /^@([a-z]*)$/i.exec(draft.trimStart())
  const mentionMenu = menu?.kind === 'mention'
  const options = mentionMenu ? crewTargets.filter((role) => role.startsWith(mention?.[1].toLowerCase() ?? ''))
    .map((role) => ({ name: role, description: role === 'team' ? 'Coordinate all four personalities' : CREW[role].title,
      role, surface: undefined }))
    : commands.filter(({ name }) => !slash || name.startsWith(slash[1].toLowerCase()))
  const menuOpen = menu?.chat === chatId
  const highlighted = menuOpen && menu.highlighted < options.length ? menu.highlighted : -1
  const optionId = (index: number) => `${menuId}-${index}`
  /// The draft store is a mirror of this state, never its source: clearing the
  /// composer writes an empty draft, which deletes the entry.
  const persistDraft = (text: string) => { if (keepDrafts) saveDraft(chatId, text) }
  const setDraft = (text: string) => {
    persistDraft(text)
    setDrafts((previous) => ({ ...previous, [chatId]: text }))
  }
  // Dictation appends instead of replacing: whatever was typed before the
  // shortcut is part of the same prompt.
  const appendDraft = (text: string) => {
    const current = drafts[chatId] ?? ''
    const next = current.trim() ? current.trimEnd() + ' ' + text : text
    persistDraft(next)
    setDrafts((previous) => ({ ...previous, [chatId]: next }))
  }
  /// Attaching picks files and writes their paths into the draft, where they are
  /// text the user can edit before sending: every harness already reads a path.
  const [attachError, setAttachError] = useState('')
  const attach = async () => {
    setAttachError('')
    try {
      const paths = await pickAttachments()
      if (paths.length) {
        setDraft(attachmentPrompt(drafts[chatId] ?? '', paths))
        input.current?.focus()
      }
    } catch (err) {
      setAttachError(errorText(err))
    }
  }
  const openMenu = (kind: 'slash' | 'mention' = 'slash') => setMenu({ chat: chatId, kind, highlighted: -1 })
  const closeMenu = () => setMenu(null)
  const openModels = () => {
    closeMenu()
    setModelMenu(chatId)
  }
  const closeModels = (focus = false) => {
    setModelMenu(null)
    if (focus) modelButton.current?.focus({ preventScroll: true })
  }

  // --- dictation -------------------------------------------------------
  const [voiceState, setVoiceState] = useState<'idle' | 'recording' | 'transcribing'>('idle')
  const [voiceLevel, setVoiceLevel] = useState(0)
  const [voiceError, setVoiceError] = useState('')
  const [installedVoices, setInstalledVoices] = useState<string[]>([])
  const voiceRecorder = useRef<MicRecorder | null>(null)
  const holding = useRef(false)
  // A saved service needs a key unless it is a server you run yourself, which
  // may take none; a local model has to be on this computer.
  const serviceProfile = voice.profiles.find((entry) => entry.id === voice.profileId)
  const serviceReady = Boolean(
    serviceProfile && (serviceProfile.key.trim() !== '' || serviceProfile.provider === 'custom'),
  )
  const voiceReady = voice.destination === 'service'
    ? serviceReady
    : voice.destination === 'local' && voice.model !== '' && installedVoices.includes(voice.model)

  useEffect(() => {
    void voiceInstalled().then(setInstalledVoices).catch(() => setInstalledVoices([]))
  }, [])

  // A model downloaded while this composer is mounted changes what the mic
  // button can do, and the window regaining focus is when that happens.
  useEffect(() => {
    const refresh = () => {
      void voiceInstalled().then(setInstalledVoices).catch(() => {})
    }
    window.addEventListener('focus', refresh)
    return () => window.removeEventListener('focus', refresh)
  }, [])

  const finishDictation = async (keep: boolean) => {
    const mic = voiceRecorder.current
    setVoiceLevel(0)
    if (!mic?.active) {
      setVoiceState('idle')
      return
    }
    if (!keep) {
      mic.cancel()
      setVoiceState('idle')
      return
    }
    try {
      const recording = await mic.stop()
      if (recording.duration < 0.25) {
        setVoiceError('That recording was too short to transcribe. Hold a little longer.')
        return
      }
      if (recording.peak < 0.01) {
        setVoiceError('The microphone picked up no sound. Check the input device in Settings, Voice.')
        return
      }
      setVoiceState('transcribing')
      const language = voice.language === 'all' ? undefined : voice.language
      const result = voice.destination === 'service' && serviceProfile
        ? await voiceTranscribeService({
            profile: serviceProfile,
            language,
            samples: recording.samples,
            sampleRate: recording.sampleRate,
            channels: recording.channels,
          })
        : await voiceTranscribe({
            modelId: voice.model,
            language,
            samples: recording.samples,
            sampleRate: recording.sampleRate,
            channels: recording.channels,
          })
      if (result.text) {
        appendDraft(result.text)
        input.current?.focus()
      } else {
        setVoiceError('Nothing was recognised in that recording.')
      }
    } catch (err) {
      setVoiceError(errorText(err))
    } finally {
      setVoiceState('idle')
    }
  }

  const startDictation = async () => {
    // The installed list can be stale - a model downloaded after this composer
    // mounted would otherwise send the user straight back to Settings.
    const installed = await voiceInstalled().catch(() => installedVoices)
    if (installed !== installedVoices) setInstalledVoices(installed)
    const ready = voice.destination === 'service'
      ? serviceReady
      : voice.destination === 'local' && voice.model !== '' && installed.includes(voice.model)
    if (!ready) {
      onOpenVoice()
      return
    }
    if (voiceState !== 'idle') return
    setVoiceError('')
    try {
      const mic = (voiceRecorder.current ??= new MicRecorder())
      await mic.start(voice.microphone, setVoiceLevel)
      setVoiceState('recording')
    } catch (err) {
      setVoiceState('idle')
      setVoiceError(err instanceof Error ? err.message : String(err))
    }
  }

  // The shortcut is registered on the window so it works wherever the caret is.
  // Re-registered per render because the handlers close over the live draft.
  useEffect(() => {
    if (!voiceReady) return
    const match = (event: KeyboardEvent) => event.code === 'Space' && event.ctrlKey && event.shiftKey
    const down = (event: KeyboardEvent) => {
      if (!match(event)) return
      event.preventDefault()
      if (event.repeat) return
      if (voice.shortcut === 'hold') {
        holding.current = true
        void startDictation()
      } else if (voiceState === 'recording') void finishDictation(true)
      else void startDictation()
    }
    const up = (event: KeyboardEvent) => {
      if (!holding.current || voice.shortcut !== 'hold') return
      if (match(event)) event.preventDefault()
      holding.current = false
      void finishDictation(true)
    }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
    }
  })

  useEffect(() => () => { voiceRecorder.current?.cancel() }, [])

  useLayoutEffect(() => {
    const el = input.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight + el.offsetHeight - el.clientHeight, 200)}px`
  }, [draft])

  const select = ({ name }: { name: string }) => {
    setDraft(mentionMenu ? `@${name} ` : `/${name}${name.endsWith(':') ? '' : ' '}`)
    closeMenu()
    input.current?.focus()
  }

  const submit = () => {
    const text = draft.trim()
    const address = addressedCrew(text)
    if (address && !address.task) {
      if (mentionMenu && options.length) select(options.find(({ name }) => name === address.mode) ?? options[0])
      else {
        setDraft('@' + address.mode)
        openMenu('mention')
        input.current?.focus()
      }
      return
    }
    if (mention && menuOpen && mentionMenu && options.length) {
      select(options[0])
      return
    }
    if (/^\/models?$/i.test(text)) {
      if (!busy) {
        setDraft('')
        if (crew) onConfigureCrew()
        else openModels()
      }
      return
    }
    if (/^\/(?:help|\?)?$/i.test(text)) {
      setDraft('/')
      openMenu()
      input.current?.focus()
      return
    }
    if (!text || ((busy || disabled) && !text.startsWith('/'))) return
    if (onSend(text)) {
      setDraft('')
      closeMenu()
    }
  }

  return (
    <div ref={composer} className="composer" onBlur={(event) => {
      if (!event.currentTarget.contains(event.relatedTarget)) closeMenu()
    }}>
      {modelOpen && <ModelPicker id={modelId} anchor={composer} agent={agent} model={model} effort={effort}
        recent={recentModels} configured={configuredModels} hasHistory={hasHistory} onSelect={onSelectModel} onClose={closeModels} />}
      {menuOpen && <div className="slash-menu">
        <ul id={menuId} role="listbox" aria-label={mentionMenu ? 'Agent mentions' : 'Slash commands'}>
          {options.map((command, index) => (
            <li key={command.name} role="presentation">
              <button type="button" id={optionId(index)} role="option" tabIndex={-1}
                aria-selected={index === highlighted}
                onMouseDown={(event) => event.preventDefault()} onClick={() => select(command)}>
                <code className={mentionMenu ? 'mention-code' : undefined}>
                  {'role' in command && command.role !== 'team' && <CrewPortrait role={command.role} />}
                  {mentionMenu ? '@' : '/'}{command.name}
                </code>
                <span>{command.description}</span>
                {command.surface === 'terminal' && <small>Nova terminal</small>}
              </button>
            </li>
          ))}
        </ul>
        {options.length === 0 && <p role="status">{mentionMenu ? 'No matching agents.' : 'No matching commands.'}</p>}
      </div>}
      <textarea ref={input} rows={1} value={draft} aria-label="Prompt"
        aria-autocomplete="list" aria-controls={menuOpen ? menuId : undefined}
        aria-activedescendant={highlighted >= 0 ? optionId(highlighted) : undefined}
        placeholder={!workspace ? 'Open a folder to begin' : `Ask ${recipient} · @ for agents · / for commands`}
        onChange={(event) => {
          const text = event.target.value
          setDraft(text)
          if (/^\/[a-z0-9_?:-]*$/i.test(text.trimStart())) openMenu()
          else if (/^@[a-z]*$/i.test(text.trimStart())) openMenu('mention')
          else closeMenu()
        }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return
          if (menuOpen && event.key === 'Escape') {
            event.preventDefault()
            event.stopPropagation()
            closeMenu()
          } else if (menuOpen && options.length && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
            event.preventDefault()
            const next = event.key === 'ArrowDown' ? (highlighted + 1) % options.length
              : highlighted <= 0 ? options.length - 1 : highlighted - 1
            setMenu({ chat: chatId, kind: mentionMenu ? 'mention' : 'slash', highlighted: next })
            document.getElementById(optionId(next))?.scrollIntoView({ block: 'nearest' })
          } else if (highlighted >= 0 && (event.key === 'Tab' || event.key === 'Enter') && !event.shiftKey) {
            event.preventDefault()
            select(options[highlighted])
          } else if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault()
            submit()
          }
        }} />
      {voiceError && <p className="composer-voice-error" role="status">
        {voiceError}
        <button type="button" className="ghost" onClick={() => { setVoiceError(''); onOpenVoice() }}>Voice settings</button>
      </p>}
      {attachError && <p className="composer-voice-error" role="status">{attachError}</p>}
      {busy && <div className="composer-activity" data-waiting={waiting} role="status">
        <span className="activity-meter" aria-hidden="true"><i /><i /><i /></span>
        <span>{work}</span>
      </div>}
      <div className="composer-meta">
        <span className="composer-tags">
          <button type="button" className="ghost voice-trigger" aria-label="Dictate a prompt"
            data-state={voiceState} aria-pressed={voiceState === 'recording'}
            disabled={busy && voiceState === 'idle'}
            title={voiceReady ? 'Dictate · ' + VOICE_SHORTCUT_LABEL : 'Set up on-device dictation in Settings, Voice'}
            onClick={() => {
              if (voiceState === 'recording') void finishDictation(true)
              else if (voiceState === 'idle') void startDictation()
            }}>
            <span className="voice-dot" aria-hidden="true" />
            {voiceState === 'recording' ? 'Listening' : voiceState === 'transcribing' ? 'Transcribing' : 'Dictate'}
          </button>
          {voiceState === 'recording' && (
            <span className="voice-meter" aria-hidden="true"><i style={{ width: Math.round(voiceLevel * 100) + '%' }} /></span>
          )}
          <button type="button" className="ghost slash-trigger" aria-label="Browse slash commands"
            aria-expanded={menuOpen && !mentionMenu} aria-controls={menuOpen && !mentionMenu ? menuId : undefined}
            onClick={() => {
              if (menuOpen && !mentionMenu) closeMenu()
              else {
                if (!draft) setDraft('/')
                openMenu()
                input.current?.focus()
              }
            }}>/</button>
          {!crew && <button type="button" ref={modelButton} className="ghost model-trigger" disabled={busy}
            aria-label="Choose agent and model" aria-haspopup="dialog" aria-expanded={modelOpen}
            aria-controls={modelOpen ? modelId : undefined} title={model ? backend + ' · ' + model : backend + ' · Harness default'}
            onClick={() => { if (modelOpen) closeModels(); else openModels() }}>
            <AgentMark id={agent} />
            <span className="model-trigger-agent">{backend}</span>
            {model && <span className="model-trigger-name">{model}</span>}
          </button>}
          <label className="crew-select" data-mode={autoApprove ? 'auto' : 'manual'}
            title={autoApprove ? 'Auto approval: ordinary tools and plans run without asking; questions still require an answer and harness restrictions remain in force.' : 'Manual approval: every tool call waits for you. Click for auto approval.'}>
            <Glyph name={autoApprove ? 'lock-open' : 'lock'} />
            <select aria-label="Permission mode" value={autoApprove ? 'auto' : 'manual'}
              onChange={(event) => onAutoApprove(event.target.value === 'auto')}>
              <option value="manual">Manual approval</option>
              <option value="auto">Auto approval</option>
            </select>
          </label>
          <label className="crew-select" data-mode={crew ?? 'none'}>
            {crew && crew !== 'team' ? <CrewPortrait role={crew} /> : <Glyph name="people" />}
            <select aria-label="Agent personality" disabled={busy} value={crew ?? ''}
              onChange={(event) => onCrew(event.target.value as CrewMode || undefined)}>
              <option value="">Personality</option>
              {crewRoles.map((role) => <option key={role} value={role}>{CREW[role].name}</option>)}
              <option value="team">Team</option>
            </select>
          </label>
          {crew && <button className="ghost" type="button" disabled={busy} onClick={onConfigureCrew}>Configure</button>}
          <span className="composer-workspace" title={workspace ?? undefined}>{workspace ?? 'no folder'}</span>
          {usage && <span className="composer-workspace" title="Input / output tokens">
            {usage.input.toLocaleString()} in · {usage.output.toLocaleString()} out
          </span>}
        </span>
        <button type="button" className="ghost composer-attach" aria-label="Attach files"
          title="Attach files — their paths are added to the prompt" onClick={() => void attach()}>
          <Glyph name="attach" />
        </button>
        {busy ? (
          <button type="button" className="stop" onClick={onStop}>Stop</button>
        ) : (
          <button type="button" className="send" onClick={submit} disabled={disabled && !draft.trimStart().startsWith('/')}>Send</button>
        )}
      </div>
    </div>
  )
}
