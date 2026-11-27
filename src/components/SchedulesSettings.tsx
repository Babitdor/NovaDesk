import { useState, type FormEvent } from 'react'

import type { WorkspaceInfo } from '../lib/events'
import { describe as describeSchedule, MINUTE, nextRun } from '../lib/scheduler'
import {
  AGENTS, type AgentId, type AutoApprove, type Preferences, type Schedule, type ScheduleSpec,
} from '../lib/settings'
import { Group, Toggle } from './SettingsRows'

type Props = {
  preferences: Preferences
  update: (patch: Partial<Preferences>) => void
  workspace: WorkspaceInfo | null
}

const agentIds = Object.keys(AGENTS) as AgentId[]

type Form = {
  kind: ScheduleSpec['kind']
  prompt: string
  agent: AgentId
  at: string
  time: string
  weekdays: boolean
  minutes: number
  steps: string
}

const emptyForm = (agent: AgentId): Form => ({
  kind: 'once', prompt: '', agent, at: '', time: '09:00', weekdays: true, minutes: 30, steps: '',
})

/// The four cards are prefills, not saved objects: pressing one fills the form so
/// the run list only ever holds schedules the user has seen the terms of.
const PRESETS: { title: string; detail: string; form: Partial<Form> }[] = [
  {
    title: 'Morning standup',
    detail: 'Weekdays at 09:00',
    form: { kind: 'daily', weekdays: true, time: '09:00', prompt: 'Summarise yesterday’s commits and today’s open changes.' },
  },
  {
    title: 'CI every 30 min',
    detail: 'Checks the pipeline and reports',
    form: { kind: 'every', minutes: 30, prompt: 'Run the project checks and report anything that fails.' },
  },
  {
    title: 'After a thread finishes',
    detail: 'One follow-up once the running task ends',
    form: { kind: 'afterThread', prompt: 'Review the task that just finished and list what is still unverified.' },
  },
  {
    title: 'Chain threads',
    detail: 'Three prompts, one after another',
    form: {
      kind: 'chain',
      steps: 'Draft the change and its tests.\nRun the project checks and fix what fails.\nSummarise the diff and what remains unverified.',
    },
  },
]

export function SchedulesSettings({ preferences: p, update, workspace }: Props) {
  const root = workspace?.root ?? ''
  const [status, setStatus] = useState('')
  const [form, setForm] = useState<Form>(() => emptyForm(p.defaultAgent))
  const set = (patch: Partial<Form>) => setForm((current) => ({ ...current, ...patch }))

  const save = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setStatus('')
    if (!root) {
      setStatus('Open a workspace to schedule work in it.')
      return
    }
    const prompt = form.prompt.trim()
    const spec = buildSpec(form)
    if (!spec) {
      setStatus('Enter a prompt and the times this schedule needs.')
      return
    }
    if (!prompt && spec.kind !== 'chain') {
      setStatus('Enter a prompt.')
      return
    }
    if (spec.kind === 'chain' && spec.steps.length === 0) {
      setStatus('Enter at least one step, one per line.')
      return
    }
    const at = firstRun(spec, form)
    if (at === null) {
      setStatus('Choose a future date and time.')
      return
    }
    const schedule: Schedule = {
      id: crypto.randomUUID(), prompt, workspace: root,
      agent: form.agent, at, daily: spec.kind === 'daily', spec, origin: 'user',
      createdAt: Date.now(), chainStep: 0,
    }
    update({ schedules: [...p.schedules, schedule] })
    setForm(emptyForm(form.agent))
    setStatus('Schedule saved.')
  }

  const remove = (id: string) => update({ schedules: p.schedules.filter((entry) => entry.id !== id) })
  const upcoming = [...p.schedules]
    .filter((schedule) => schedule.workspace === root)
    .sort((a, b) => rank(a) - rank(b))

  return (
    <>
      <Group title="Agent scheduling">
        <Toggle title="Let agents schedule wake-ups"
          detail="A task may ask Nova to start another one later. Off drops those requests with a notice."
          checked={p.agentsMaySchedule} onChange={(agentsMaySchedule) => update({ agentsMaySchedule })} />
        <Toggle title="Ask before agents message other threads"
          detail="Cross-thread starts from an agent wait for your approval in the task it came from."
          checked={p.askBeforeCrossThread} onChange={(askBeforeCrossThread) => update({ askBeforeCrossThread })} />
        <Toggle title="Pause all schedules" detail="Existing runs keep going; nothing new starts."
          checked={p.pauseSchedules} onChange={(pauseSchedules) => update({ pauseSchedules })} />
        <label className="settings-row">
          <span><strong>Agent limit per thread</strong>
            <small>Concurrent agent-originated runs plus queued wake-ups one task may hold.
              Your own turns stay one at a time.</small></span>
          <input type="number" min={1} max={100} value={p.agentLimit} aria-label="Agent limit per thread"
            onChange={(event) => update({
              agentLimit: Math.min(100, Math.max(1, Math.round(Number(event.target.value) || 1))),
            })} />
        </label>
        {/* One choice per kind of interrupt, and the default answers none of
            them: widening what runs without asking is the user's call. */}
        <label className="settings-row">
          <span><strong>Auto-approve tool permissions</strong>
            <small>Tool permission prompts are approved for you, and the task records that it
              happened. A plan still asks unless you choose the last option, and a question
              always asks.</small></span>
          <select value={p.autoApprove} aria-label="Auto-approve tool permissions"
            onChange={(event) => update({ autoApprove: event.target.value as AutoApprove })}>
            <option value="off">Off</option>
            <option value="tools">Tools only</option>
            <option value="all">Tools and plans</option>
          </select>
        </label>
      </Group>

      <Group title="New schedule">
        <div className="settings-presets" aria-label="Schedule presets">
          {PRESETS.map((preset) => (
            <button key={preset.title} type="button" onClick={() => {
              set(preset.form)
              setStatus(preset.title + ' filled in below — review and save it.')
            }}>
              <span>{preset.title}<small>{preset.detail}</small></span>
            </button>
          ))}
        </div>
        <form className="settings-convert" aria-label="Schedule a prompt" onSubmit={save}>
          <label>Runs
            <select value={form.kind} onChange={(event) => set({ kind: event.target.value as Form['kind'] })}>
              <option value="once">Once</option>
              <option value="daily">Every day</option>
              <option value="every">On an interval</option>
              <option value="afterThread">After the running task finishes</option>
              <option value="chain">A chain of prompts</option>
            </select>
          </label>
          {form.kind === 'once' && <label>Run at
            <input type="datetime-local" value={form.at} required
              onChange={(event) => set({ at: event.target.value })} />
          </label>}
          {(form.kind === 'daily') && <>
            <label>Time<input type="time" value={form.time} required
              onChange={(event) => set({ time: event.target.value })} /></label>
            <label className="settings-inline"><input type="checkbox" checked={form.weekdays}
              onChange={(event) => set({ weekdays: event.target.checked })} /> Weekdays only</label>
          </>}
          {form.kind === 'every' && <label>Every (minutes)
            <input type="number" min={1} max={1440} value={form.minutes}
              onChange={(event) => set({ minutes: Math.min(1440, Math.max(1, Math.round(Number(event.target.value) || 1))) })} />
          </label>}
          {form.kind !== 'chain' && <label>Prompt
            <textarea rows={3} maxLength={12000} disabled={!root} value={form.prompt}
              onChange={(event) => set({ prompt: event.target.value })} />
          </label>}
          {form.kind === 'chain' && <label>Steps, one per line
            <textarea rows={5} maxLength={12000} disabled={!root} value={form.steps}
              onChange={(event) => set({ steps: event.target.value })} />
          </label>}
          <label>Agent<select value={form.agent} onChange={(event) => set({ agent: event.target.value as AgentId })}>
            {agentIds.map((id) => <option key={id} value={id}>{AGENTS[id].name}</option>)}
          </select></label>
          <div className="settings-action">
            <button type="submit" disabled={!root}>Save schedule</button>
          </div>
        </form>
        <p className="settings-help">
          A schedule runs while Nova Desktop is open and the same folder is active. Nothing runs in the
          background after the window closes.
        </p>
      </Group>

      <Group title="Upcoming">
        {!root && <p className="settings-help">Open a workspace to see its schedules.</p>}
        {root && upcoming.length === 0 && <p className="settings-help">No schedules for this folder yet.</p>}
        {upcoming.map((schedule) => (
          <div key={schedule.id} className="settings-row">
            <span><strong>{schedulePromptLabel(schedule)}</strong>
              <small>{describeSchedule(schedule)} · {AGENTS[schedule.agent]?.name ?? schedule.agent}
                {schedule.origin !== 'user' ? ' · from an agent' : ''}</small></span>
            <button type="button" onClick={() => remove(schedule.id)}>Remove</button>
          </div>
        ))}
      </Group>

      <Group title="Activity">
        {p.scheduleActivity.length === 0 && <p className="settings-help">No runs yet.</p>}
        {p.scheduleActivity.map((entry) => (
          <div key={entry.id} className="settings-row">
            <span><strong>{entry.detail}</strong>
              <small>{new Date(entry.at).toLocaleString()} · {entry.status}</small></span>
          </div>
        ))}
        {p.scheduleActivity.length > 0 && <div className="settings-action">
          <button type="button" onClick={() => update({ scheduleActivity: [] })}>Clear activity</button>
        </div>}
      </Group>

      <p className="settings-status" role="status">{status}</p>
    </>
  )
}

function schedulePromptLabel(schedule: Schedule): string {
  const prompt = schedule.spec.kind === 'chain'
    ? schedule.spec.steps[schedule.chainStep ?? 0] ?? schedule.prompt
    : schedule.prompt
  return prompt.slice(0, 80) || 'Chain'
}

/// `null` means the form is incomplete in a way that cannot be repaired by
/// picking a time, so the caller reports one message instead of guessing.
function buildSpec(form: Form): ScheduleSpec | null {
  switch (form.kind) {
    case 'once':
      return new Date(form.at).getTime() > Date.now() ? { kind: 'once' } : null
    case 'daily':
      return /^\d{2}:\d{2}$/.test(form.time) ? { kind: 'daily', weekdays: form.weekdays } : null
    case 'every':
      return Number.isFinite(form.minutes) && form.minutes >= 1 ? { kind: 'every', minutes: form.minutes } : null
    case 'afterThread':
      return { kind: 'afterThread' }
    case 'chain': {
      const steps = form.steps.split('\n').map((step) => step.trim()).filter(Boolean).slice(0, 20)
      return steps.length ? { kind: 'chain', steps } : null
    }
  }
}

/// The first time the spec fires, from the form. `every` starts one interval out
/// so saving a form never fires immediately.
function firstRun(spec: ScheduleSpec, form: Form): number | null {
  const now = Date.now()
  switch (spec.kind) {
    case 'once':
      return new Date(form.at).getTime()
    case 'daily': {
      const [hours, minutes] = form.time.split(':').map(Number)
      const next = new Date()
      next.setHours(hours, minutes, 0, 0)
      let at = next.getTime()
      if (at <= now) at = nextRun(spec, at, now) ?? at
      if (spec.weekdays) {
        for (let guard = 0; guard < 8 && [0, 6].includes(new Date(at).getDay()); guard += 1) {
          at = nextRun(spec, at, now) ?? at
        }
      }
      return at
    }
    case 'every':
      return now + spec.minutes * MINUTE
    case 'afterThread':
      // Armed, not timed: the ticker starts it once a task has finished.
      return 0
    case 'chain':
      return now
  }
}

function rank(schedule: Schedule): number {
  if (schedule.spec.kind === 'afterThread') return Number.MAX_SAFE_INTEGER
  return schedule.at
}
