import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type RefObject } from 'react'

import { invoke } from '@tauri-apps/api/core'
import novaModels from '../lib/nova-models.json'
import { AGENTS, type AgentId, type ModelSelection, type Preferences } from '../lib/settings'
import { AgentMark } from './AgentMark'

type Model = { id: string; label: string; context?: string | null; efforts?: string[] | null }
type Row = ModelSelection & { label: string; context?: string | null }
type Scope = AgentId | 'recent' | 'all'

const agents = Object.keys(AGENTS) as AgentId[]
const PAGE_SIZE = 80
const dynamic: AgentId[] = ['pi', 'codex', 'opencode', 'commandcode', 'hermes']
const builtIn: Partial<Record<AgentId, Model[]>> = {
  nova: novaModels.map((id) => ({ id, label: id.slice(id.indexOf(':') + 1) })),
  claude: [
    { id: 'sonnet', label: 'Claude Sonnet' },
    { id: 'opus', label: 'Claude Opus' },
    { id: 'haiku', label: 'Claude Haiku' },
    { id: 'fable', label: 'Claude Fable' },
  ],
}

const effortOptions: Partial<Record<AgentId, string[]>> = {
  pi: ['off', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max'],
  claude: ['low', 'medium', 'high', 'xhigh', 'max'],
  codex: ['low', 'medium', 'high', 'xhigh'],
  commandcode: ['low', 'medium', 'high'],
}
const effortLabels: Record<string, string> = {
  off: 'Off', none: 'None', minimal: 'Minimal', low: 'Low', medium: 'Medium',
  high: 'High', xhigh: 'Extra high', max: 'Max', ultra: 'Ultra',
}
const windowNumber = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })

const cache = new Map<AgentId, { until: number; request: Promise<Model[]> }>()

function discover(agent: AgentId) {
  const entry = cache.get(agent)
  if (entry && Date.now() < entry.until) return entry.request
  const request = invoke<Model[]>('agent_models', { agent }).catch(() => {
    cache.delete(agent)
    return []
  })
  cache.set(agent, { until: Date.now() + 3 * 60 * 1000, request })
  return request
}

function windowLabel(context: string) {
  return /^\d+$/.test(context) ? windowNumber.format(Number(context)) : context
}

type Props = {
  id: string
  anchor: RefObject<HTMLDivElement | null>
  agent: AgentId
  model?: string
  effort?: string
  recent: ModelSelection[]
  configured: Preferences['models']
  hasHistory: boolean
  onSelect: (selection: ModelSelection) => void
  onClose: (restoreFocus?: boolean) => void
}

export function ModelPicker({ id, anchor, agent, model, effort, recent, configured, hasHistory, onSelect, onClose }: Props) {
  const panel = useRef<HTMLDivElement>(null)
  const search = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const [scope, setScope] = useState<Scope>(agent)
  const [query, setQuery] = useState('')
  const [limit, setLimit] = useState(PAGE_SIZE)
  const [catalog, setCatalog] = useState<Partial<Record<AgentId, Model[]>>>({})
  const needle = query.trim().toLowerCase()
  const provider = scope === 'all' || scope === 'recent' ? null : scope

  useLayoutEffect(() => {
    const popup = panel.current
    const target = anchor.current
    if (!popup || !target) return
    const place = () => {
      const rect = target.getBoundingClientRect()
      const above = rect.top - 12
      const below = window.innerHeight - rect.bottom - 12
      const useAbove = above >= 340 || above >= below
      popup.style.left = rect.left + 'px'
      popup.style.width = Math.min(rect.width, 540) + 'px'
      popup.style.maxHeight = Math.min(470, useAbove ? above : below) + 'px'
      popup.style.top = useAbove ? 'auto' : rect.bottom + 5 + 'px'
      popup.style.bottom = useAbove ? window.innerHeight - rect.top + 5 + 'px' : 'auto'
    }
    place()
    if (!popup.matches(':popover-open')) popup.showPopover()
    search.current?.focus()
    const observer = new ResizeObserver(place)
    observer.observe(target)
    if (target.parentElement) observer.observe(target.parentElement)
    window.addEventListener('resize', place)
    return () => {
      observer.disconnect()
      window.removeEventListener('resize', place)
    }
  }, [anchor])

  useEffect(() => {
    const wanted = scope === 'all' ? dynamic : provider && dynamic.includes(provider) ? [provider] : []
    let stale = false
    for (const next of wanted) void discover(next).then((models) => {
      if (!stale) setCatalog((previous) => ({ ...previous, [next]: models }))
    })
    return () => { stale = true }
  }, [scope, provider])

  const modelsFor = (next: AgentId) => [...(builtIn[next] ?? []), ...(catalog[next] ?? [])]
  const levelOptions = (next: AgentId, id: string | null) =>
    modelsFor(next).find((entry) => entry.id === id)?.efforts ?? effortOptions[next] ?? []
  const rows: Row[] = scope === 'recent'
    ? recent.map((entry) => ({ ...entry, label: entry.model ?? 'Harness default' }))
    : (provider ? [provider] : agents).flatMap((next) => {
      const choices = modelsFor(next)
      const extra = new Set([next === agent ? model : undefined, configured[next]?.trim()])
      return [
        { agent: next, model: null, label: 'Harness default' },
        ...Array.from(extra).flatMap((id) => id && !choices.some((entry) => entry.id === id)
          ? [{ agent: next, model: id, label: id }] : []),
        ...choices.map((entry) => ({ agent: next, model: entry.id, label: entry.label, context: entry.context })),
      ]
    })
  const visible = rows.filter((row) =>
    [row.label, row.model, AGENTS[row.agent].name].some((value) => value?.toLowerCase().includes(needle)))
  const pending = scope === 'all' ? dynamic.some((id) => catalog[id] === undefined)
    : !!provider && dynamic.includes(provider) && catalog[provider] === undefined
  const typed = query.trim()
  const custom = provider && typed.length <= 200 && !typed.startsWith('-') && /^[^\s\p{Cc}]+$/u.test(typed)
    && (provider !== 'nova' || /^[^:]+:.+$/.test(typed)) && !rows.some((row) => row.model === typed)
  const choices = provider && provider !== agent ? [] : levelOptions(agent, model ?? null)

  const choose = (row: ModelSelection) => {
    onSelect({
      ...row,
      effort: scope === 'recent' ? row.effort
        : row.agent === agent && row.model === (model ?? null) ? effort : undefined,
    })
  }

  const chooseScope = (next: Scope) => {
    setScope(next)
    setQuery('')
    setLimit(PAGE_SIZE)
    list.current?.scrollTo(0, 0)
  }

  const navigate = (event: KeyboardEvent) => {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      onClose(true)
      return
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    if (event.target !== search.current && !list.current?.contains(event.target as Node)) return
    const buttons = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('button') ?? [])
    if (!buttons.length) return
    event.preventDefault()
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement)
    buttons[event.key === 'ArrowDown' ? (at + 1) % buttons.length : at <= 0 ? buttons.length - 1 : at - 1].focus()
  }

  return (
    <div id={id} ref={panel} popover="auto" className="model-picker" role="dialog" aria-label="Select agent and model"
      onToggle={(event) => { if (event.newState === 'closed') onClose() }} onKeyDown={navigate}>
      <div className="model-picker-search">
        <input ref={search} type="search" aria-label="Search models" placeholder="Search models or enter an ID…"
          value={query}
          onChange={(event) => { setQuery(event.target.value); setLimit(PAGE_SIZE); list.current?.scrollTo(0, 0) }}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.nativeEvent.isComposing) return
            const exact = visible.find(({ model }) => model?.toLowerCase() === needle)
            if (exact) choose(exact)
            else if (provider && custom && !visible.length) choose({ agent: provider, model: typed })
            else list.current?.querySelector('button')?.focus()
          }} />
        <button type="button" className="ghost" aria-label="Close model picker" onClick={() => onClose(true)}>×</button>
      </div>
      <div className="model-picker-body">
        <nav aria-label="Agent harnesses" className="model-picker-harnesses">
          <button type="button" aria-pressed={scope === 'recent'} onClick={() => chooseScope('recent')}>Recent</button>
          <button type="button" aria-pressed={scope === 'all'} onClick={() => chooseScope('all')}>All models</button>
          {agents.map((id) => <button key={id} type="button" aria-pressed={scope === id} onClick={() => chooseScope(id)}>
            <AgentMark id={id} />
            {AGENTS[id].name}
          </button>)}
        </nav>
        <div className="model-picker-results" ref={list} aria-label="Model results">
          {provider !== agent && hasHistory && <p>Switching harnesses starts a new session for the next message.</p>}
          {visible.slice(0, limit).map((row) => (
            <button type="button" key={row.agent + ':' + row.model}
              aria-pressed={row.agent === agent && row.model === (model ?? null)} onClick={() => choose(row)}>
              <span><strong>{row.label}</strong>
                {(scope === 'all' || scope === 'recent' || row.model !== row.label) && <small>
                  {scope === 'all' || scope === 'recent'
                    ? <><AgentMark id={row.agent} /> {AGENTS[row.agent].name}</>
                    : row.model}
                </small>}
              </span>
              {row.context && <span className="model-context" title="Context window">{windowLabel(row.context)}</span>}
            </button>
          ))}
          {visible.length > limit && <button type="button" onClick={() => setLimit(limit + PAGE_SIZE)}>Show more models</button>}
          {provider && custom && <button type="button" onClick={() => choose({ agent: provider, model: typed })}>
            <span><strong>Use custom model</strong><small>{typed}</small></span>
          </button>}
          {pending && <p role="status">Loading models…</p>}
          {!pending && !visible.length && !custom && <p>{scope === 'recent' && !needle ? 'Your selected models appear here.' : 'No matching models.'}</p>}
          {provider && dynamic.includes(provider) && !pending && !catalog[provider]?.length && !needle
            && <p>The CLI catalog is unavailable. Use its default or enter a model ID.</p>}
        </div>
      </div>
      {(!provider || provider === agent) && <footer className="model-picker-effort">
        {choices.length > 0 ? <>
          <span>Effort</span>
          <div role="group" aria-label="Reasoning effort">
            <button type="button" aria-pressed={!effort} onClick={() => onSelect({ agent, model: model ?? null })}>Default</button>
            {choices.map((level) => (
              <button type="button" key={level} aria-pressed={effort === level} onClick={() => onSelect({ agent, model: model ?? null, effort: level })}>
                {effortLabels[level] ?? level}
              </button>
            ))}
          </div>
        </> : <small>{agent === 'opencode' ? 'Set a variant with provider/model#variant.'
          : agent === 'nova' ? 'Nova uses its saved /effort setting.' : 'Reasoning uses the harness or model default.'}</small>}
      </footer>}
    </div>
  )
}
