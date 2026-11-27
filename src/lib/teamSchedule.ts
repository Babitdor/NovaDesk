import type { TeamStage } from './crew'

/// Writers run alone. A parked coordinator reserves one slot during investigation.
export function nextTeamBatch(stages: TeamStage[], limit: number): number[] {
  if (stages.some((stage) => stage.status === 'running')) return []
  const first = stages.findIndex((stage) => stage.status === 'pending')
  if (first < 0) return []
  if (!stages[first].readOnly) return [first]
  const batch = [first]
  for (let at = first + 1; at < stages.length && batch.length < Math.max(1, limit - 1); at++) {
    if (stages[at].status !== 'pending') continue
    if (!stages[at].readOnly) break
    batch.push(at)
  }
  return batch
}

export function recoverTeamRun<T extends { stages: TeamStage[]; finishedAt?: number; awaitingPlan?: number | null }>(run: T): T & { interrupted?: boolean } {
  if (run.finishedAt) return run
  return { ...run, interrupted:true, awaitingPlan:null,
    stages:run.stages.map((stage) => stage.status === 'running' || stage.status === 'pending'
      ? { ...stage, status:'failed' as const } : stage) }
}
