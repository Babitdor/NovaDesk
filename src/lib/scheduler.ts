import type { AgentId, Schedule, ScheduleSpec } from './settings'

/// The schedule engine, kept free of React and IPC so it can be tested with
/// `node --test` and reasoned about without a window. Everything here is pure:
/// the caller supplies the clock, which is what makes "a due run fires once"
/// a property of the data instead of a side effect of an interval.

export const MINUTE = 60_000
export const DAY = 86_400_000

const WEEKEND = new Set([0, 6])

export function isSpec(value: unknown): value is ScheduleSpec {
  if (!value || typeof value !== 'object') return false
  const spec = value as { kind?: unknown; weekdays?: unknown; minutes?: unknown; steps?: unknown }
  switch (spec.kind) {
    case 'once':
    case 'afterThread':
      return true
    case 'daily':
      return spec.weekdays === undefined || typeof spec.weekdays === 'boolean'
    case 'every':
      return typeof spec.minutes === 'number' && Number.isFinite(spec.minutes)
        && spec.minutes >= 1 && spec.minutes <= 24 * 60
    case 'chain':
      return Array.isArray(spec.steps) && spec.steps.length > 0 && spec.steps.length <= 20
        && spec.steps.every((step) => typeof step === 'string' && step.trim() !== '')
    default:
      return false
  }
}

/// The next time this spec should fire, or null when it is event-driven or spent.
///
/// `from` is the moment the previous run belonged to, `now` is the wall clock:
/// an `every` schedule that was due while the app was closed resumes on the next
/// aligned boundary instead of firing a burst of missed runs.
export function nextRun(spec: ScheduleSpec, from: number, now = from): number | null {
  switch (spec.kind) {
    case 'once':
    case 'afterThread':
    case 'chain':
      return null
    case 'daily': {
      let next = addDays(from, 1)
      if (spec.weekdays) {
        for (let guard = 0; guard < 8 && WEEKEND.has(new Date(next).getDay()); guard += 1) {
          next = addDays(next, 1)
        }
      }
      return next
    }
    case 'every': {
      const step = Math.max(1, Math.round(spec.minutes)) * MINUTE
      const missed = Math.floor(Math.max(0, now - from) / step) + 1
      return from + missed * step
    }
  }
}

function addDays(at: number, days: number): number {
  const date = new Date(at)
  date.setDate(date.getDate() + days)
  return date.getTime()
}

/// Schedules whose time has come, for one workspace, oldest first. An
/// `afterThread` entry is not due by the clock: the turn that finishes starts it.
/// A chain is due at its own time like any other entry; `advance` moves it on.
export function dueSchedules(schedules: Schedule[], workspace: string, now: number): Schedule[] {
  return schedules
    .filter((schedule) => schedule.workspace === workspace && schedule.at > 0
      && schedule.at <= now && schedule.spec.kind !== 'afterThread')
    .sort((a, b) => a.at - b.at)
}

/// The run that happens after the task named when it was created has finished.
export function dueAfterThread(schedules: Schedule[], workspace: string, lastFinish: number): Schedule[] {
  return schedules.filter((schedule) => schedule.workspace === workspace
    && schedule.spec.kind === 'afterThread' && lastFinish >= schedule.createdAt)
}

/// What to store after a run: the next `at` (and chain progress), or null when
/// the schedule is finished and should be dropped.
export function advance(schedule: Schedule, now: number): { at: number; chainStep?: number } | null {
  const spec = schedule.spec
  switch (spec.kind) {
    case 'once':
    case 'afterThread':
      return null
    case 'daily':
    case 'every':
      return { at: nextRun(spec, schedule.at, now) ?? 0 }
    case 'chain': {
      const step = (schedule.chainStep ?? 0) + 1
      // A chain is a sequence of whole tasks, so the next prompt waits a beat
      // instead of starting inside the previous turn's tail.
      return step < spec.steps.length ? { at: now + MINUTE, chainStep: step } : null
    }
  }
}

/// The prompt this run uses. A chain steps through its list; everything else has
/// one prompt.
export function promptFor(schedule: Schedule): string {
  return schedule.spec.kind === 'chain'
    ? schedule.spec.steps[schedule.chainStep ?? 0] ?? schedule.prompt
    : schedule.prompt
}

/// Agent-originated work is capped per task: the ceiling counts what is already
/// running plus what is waiting, so a queue cannot grow without bound either.
export function withinLimit(active: number, queued: number, limit: number): boolean {
  return active + queued < Math.max(1, Math.min(100, Math.round(limit)))
}

/// One line the Schedules pane can print without the caller re-deriving it.
export function describe(schedule: Schedule): string {
  const spec = schedule.spec
  switch (spec.kind) {
    case 'once':
      return 'Once · ' + new Date(schedule.at).toLocaleString()
    case 'daily':
      return (spec.weekdays ? 'Weekdays' : 'Daily') + ' · ' + timeOfDay(schedule.at)
    case 'every':
      return 'Every ' + spec.minutes + ' min'
    case 'afterThread':
      return 'After the running task finishes'
    case 'chain': {
      const step = Math.min((schedule.chainStep ?? 0) + 1, spec.steps.length)
      return 'Chain · step ' + step + ' of ' + spec.steps.length
    }
  }
}

function timeOfDay(at: number): string {
  return new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

/// Reads whatever an older build saved. A v1 entry has no `spec` at all: `at`
/// plus `daily` is the whole model, so those two fields are the migration.
export function migrateSchedules(raw: unknown): Schedule[] {
  if (!Array.isArray(raw)) return []
  return raw.flatMap((entry): Schedule[] => {
    if (!entry || typeof entry !== 'object') return []
    const item = entry as Partial<Schedule> & { at?: unknown; daily?: unknown; agent?: unknown }
    const prompt = typeof item.prompt === 'string' ? item.prompt : ''
    const workspace = typeof item.workspace === 'string' ? item.workspace : ''
    if (!prompt.trim() || !workspace) return []
    const at = typeof item.at === 'number' && Number.isFinite(item.at) ? item.at : 0
    const spec: ScheduleSpec = isSpec(item.spec)
      ? item.spec
      : item.daily === true ? { kind: 'daily', weekdays: false } : { kind: 'once' }
    const origin = item.origin === 'agent' || item.origin === 'preset' ? item.origin : 'user'
    const chainStep = typeof item.chainStep === 'number' && item.chainStep >= 0
      ? Math.floor(item.chainStep) : 0
    return [{
      id: typeof item.id === 'string' && item.id ? item.id : crypto.randomUUID(),
      prompt,
      workspace,
      agent: (typeof item.agent === 'string' ? item.agent : 'nova') as AgentId,
      at,
      daily: spec.kind === 'daily',
      spec,
      origin,
      createdAt: typeof item.createdAt === 'number' && Number.isFinite(item.createdAt)
        ? item.createdAt : at || Date.now(),
      chainStep,
    }]
  })
}
