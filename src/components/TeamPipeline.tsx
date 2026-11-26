import { useEffect, useState } from 'react'

import { CREW, type StageStatus, type TeamRun } from '../lib/crew'
import { CrewPortrait } from './CrewPortrait'

const STATUS_LABEL: Record<StageStatus, string> = {
  pending: 'Queued', running: 'Running', done: 'Done', failed: 'Failed', skipped: 'Skipped',
}

const formatDuration = (ms: number) => {
  const seconds = Math.max(0, Math.round(ms / 1000))
  return seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`
}

const formatTokens = (count: number) =>
  count >= 10_000 ? `${(count / 1000).toFixed(1)}k` : String(count)

/// The @team pipeline strip: every stage of the run with its live status, the
/// plan approval gate, and the actions a finished run can still take. Sits
/// above the composer, so the state of the team is visible while it works.
export function TeamPipeline({ run, busy, onDecision, onRetry }: {
  run: TeamRun
  busy: boolean
  onDecision: (decision: 'approve' | 'skip' | string) => void
  onRetry: () => void
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState('')
  /// A live clock so the running stage's elapsed time moves without the chat
  /// stream having to repaint it.
  const [clock, setClock] = useState(() => Date.now())

  useEffect(() => {
    if (!busy) return
    const timer = setInterval(() => setClock(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [busy])

  const now = busy ? clock : run.finishedAt ?? clock
  const failed = run.stages.filter((stage) => stage.status === 'failed').length
  const plan = run.awaitingPlan != null ? run.handoffs[run.awaitingPlan - 1]?.text : undefined

  return <aside className="team-pipeline" aria-label="Team run progress">
    <header className="team-pipeline-head">
      <strong>Team run</strong>
      <span className="team-pipeline-time">
        {busy ? formatDuration(now - run.startedAt) : run.finishedAt ? formatDuration(run.finishedAt - run.startedAt) : ''}
      </span>
    </header>
    <div className="team-stages">
      {run.stages.map((stage, index) => (
        <button key={index} type="button"
          className={`team-stage${stage.stageId ? '' : ' team-stage-plain'}`}
          disabled={!stage.stageId}
          title={stage.stageId ? 'Show this stage in the transcript' : CREW[stage.role].title}
          onClick={() => document
            .querySelector(`[data-stage="${stage.stageId}"]`)
            ?.scrollIntoView({ block: 'start', behavior: 'smooth' })}>
          <CrewPortrait role={stage.role} />
          <span className="team-stage-name">{CREW[stage.role].name}</span>
          {stage.role === 'ralph' && stage.iteration > 1 && <small>×{stage.iteration}</small>}
          <span className={`badge${stage.status === 'done' ? ' ok' : stage.status === 'failed' ? ' bad' : ''}`}>
            {stage.status === 'running'
              ? formatDuration(now - (stage.startedAt ?? now))
              : STATUS_LABEL[stage.status]}
          </span>
        </button>
      ))}
    </div>
    {plan !== undefined && <div className="team-plan approval">
      <div className="approval-detail">
        <strong>{CREW.nova.name}'s plan — approve before the team starts?</strong>
        <div className="reply-plain team-plan-text">{plan}</div>
      </div>
      {editing
        ? <div className="team-plan-edit">
            <textarea rows={5} value={draft} autoFocus
              placeholder="Rewrite the plan, or add constraints the team must respect…"
              onChange={(event) => setDraft(event.target.value)} />
            <div className="approval-actions">
              <button type="button" className="approve" disabled={!draft.trim()}
                onClick={() => {
                  setEditing(false)
                  onDecision(draft)
                }}>Send edited plan</button>
              <button type="button" className="ghost" onClick={() => setEditing(false)}>Back</button>
            </div>
          </div>
        : <div className="approval-actions">
            <button type="button" className="approve" onClick={() => onDecision('approve')}>Approve</button>
            <button type="button" onClick={() => {
              setDraft('')
              setEditing(true)
            }}>Edit</button>
            <button type="button" className="ghost" onClick={() => onDecision('skip')}>Run as planned</button>
          </div>}
    </div>}
    <footer className="team-pipeline-foot">
      <small>
        {formatTokens(run.usage.input)} in · {formatTokens(run.usage.output)} out
        {failed > 0 && ` · ${failed} stage${failed > 1 ? 's' : ''} failed`}
      </small>
      {!busy && run.finishedAt && failed > 0 && (
        <button type="button" onClick={onRetry}>Retry failed stages</button>
      )}
    </footer>
  </aside>
}
