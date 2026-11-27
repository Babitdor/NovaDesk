import { test } from 'node:test'
import assert from 'node:assert/strict'
import { nextTeamBatch, recoverTeamRun } from './teamSchedule.ts'
import type { TeamStage } from './crew'

const stages: TeamStage[] = [
  { role:'nora', status:'pending', iteration:1, readOnly:true },
  { role:'serena', status:'pending', iteration:1, readOnly:true },
  { role:'ralph', status:'pending', iteration:1 },
]
test('read-only investigations overlap with a coordinator slot reserved', () => {
  assert.deepEqual(nextTeamBatch(stages, 3), [0,1])
  assert.deepEqual(nextTeamBatch(stages, 2), [0])
  assert.deepEqual(nextTeamBatch(stages, 1), [0])
  assert.deepEqual(nextTeamBatch([{ ...stages[0], status:'running' }, ...stages.slice(1)], 5), [])
  assert.deepEqual(nextTeamBatch([{ ...stages[0], readOnly:false }, ...stages.slice(1)], 5), [0])
})
test('restart requires explicit retry, preserving completed work and assignments', () => {
  const recovered = recoverTeamRun({ stages:[{ ...stages[0], status:'done' as const }, ...stages.slice(1)] })
  assert.equal(recovered.interrupted, true)
  assert.deepEqual(recovered.stages.map((stage) => stage.status), ['done','failed','failed'])
  assert.deepEqual(nextTeamBatch(recovered.stages, 5), [])
})
