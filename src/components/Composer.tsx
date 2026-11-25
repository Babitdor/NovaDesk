import { useId, useLayoutEffect, useRef, useState } from 'react'

import type { SlashCommand } from '../lib/slash'
import { AGENTS, type AgentId, type ModelSelection, type Preferences } from '../lib/settings'
import { ModelPicker } from './ModelPicker'
import { CREW, addressedCrew, crewRoles, crewTargets, type CrewMode } from '../lib/crew'
import { CrewPortrait } from './CrewPortrait'

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
  onSend: (prompt: string) => boolean
  onStop: () => void
}

export function Composer({
  chatId, commands, busy, disabled, agent, model, effort, recentModels, configuredModels, onSelectModel,
  hasHistory, work, waiting, obscured, crew, onCrew, onConfigureCrew, usage, workspace, onSend, onStop,
}: Props) {
  const composer = useRef<HTMLDivElement>(null)
  const modelButton = useRef<HTMLButtonElement>(null)
  const input = useRef<HTMLTextAreaElement>(null)
  const menuId = useId()
  const modelId = useId()
  const [drafts, setDrafts] = useState<Record<string, string>>({})
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
  const setDraft = (text: string) => setDrafts((previous) => ({ ...previous, [chatId]: text }))
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
      {busy && <div className="composer-activity" data-waiting={waiting} role="status">
        <span className="activity-meter" aria-hidden="true"><i /><i /><i /></span>
        <span>{work}</span>
      </div>}
      <div className="composer-meta">
        <span className="composer-tags">
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
            <span className="model-trigger-agent">{backend}</span>
            {model && <span className="model-trigger-name">{model}</span>}
            <span aria-hidden="true">⌄</span>
          </button>}
          <label className="crew-select">
            {crew && crew !== 'team' && <CrewPortrait role={crew} />}
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
        {busy ? (
          <button type="button" className="stop" onClick={onStop}>Stop</button>
        ) : (
          <button type="button" className="send" onClick={submit} disabled={disabled && !draft.trimStart().startsWith('/')}>Send</button>
        )}
      </div>
    </div>
  )
}
