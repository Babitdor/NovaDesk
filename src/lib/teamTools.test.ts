import assert from 'node:assert/strict'
import { test } from 'node:test'
import { crewPrompt } from './crew.ts'
import { loadPreferences } from './settings.ts'
import { nextTeamBatch } from './teamSchedule.ts'
import { teamTools, type CrewControl } from './teamTools.ts'

function fixture() {
  const preferences = loadPreferences()
  preferences.agentLimit = 3
  preferences.crew = { nora:{ agent:'codex' }, serena:{ agent:'codex' } }
  const run: CrewControl = { id:'run', chatId:'chat', cancelled:false, delegated:false, notes:[], team:{ id:'run', task:'task', startedAt:0,
    stages:[{ role:'nova', status:'running', iteration:1 }, { role:'ralph', status:'pending', iteration:1 }, { role:'nova', status:'pending', iteration:1 }], handoffs:[], usage:{ input:0, output:0 } } }
  const messages: string[] = []
  const call = teamTools({ preferences, run:() => run, patch:(_, patch) => { run.team = patch(run.team) }, message:(_, text) => messages.push(text) })
  const args = { _runId:'run', _sender:'nova', _harness:'nova' }
  return { call, args, run, preferences, messages }
}

test('bounded delegation replaces fallback workers and schedules only enforced readers together', () => {
  const { call, args, run } = fixture()
  call('team_delegate', { ...args, role:'nora', task:'Investigate definitions', investigate:true })
  call('team_delegate', { ...args, role:'serena', task:'Investigate callers', investigate:true })
  assert.deepEqual(run.team.stages.map((stage) => stage.role), ['nova','nora','serena','nova'])
  assert.deepEqual(nextTeamBatch(run.team.stages, 3), [])
  run.team.stages[0].status = 'done'
  assert.deepEqual(nextTeamBatch(run.team.stages, 3), [1,2])
  assert.ok(run.team.stages.slice(1,3).every((stage) => stage.assignmentId && stage.harness === 'codex' && stage.readOnly))
})

test('messages preserve sender, recipient and harness, and reach only the addressed prompt', () => {
  const { call, args, run, messages } = fixture()
  call('team_message', { ...args, _sender:'nora', _harness:'pi', to:'serena', text:'Check the lock owner' })
  assert.equal(run.team.handoffs[0].to, 'serena')
  assert.equal(run.team.handoffs[0].harness, 'pi')
  assert.match(messages[0], /Nora → Serena · pi/)
  assert.match(crewPrompt('serena','task','',run.notes,true,false), /Check the lock owner/)
  assert.doesNotMatch(crewPrompt('ralph','task','',run.notes,true,false), /Check the lock owner/)
})

test('wrong runs, worker delegation, disabled members and exhausted limits are rejected', () => {
  const { call, args, run, preferences } = fixture()
  assert.throws(() => call('team_status', { ...args, _runId:'other' }), /no longer active/)
  assert.throws(() => call('team_delegate', { ...args, _sender:'nora', role:'serena', task:'edit' }), /Invalid/)
  preferences.team.roles.nora = false
  assert.throws(() => call('team_delegate', { ...args, role:'nora', task:'read' }), /disabled/)
  preferences.agentLimit = 1
  call('team_delegate', { ...args, role:'serena', task:'edit' })
  assert.equal(run.team.stages[1].readOnly, false)
  assert.throws(() => call('team_follow_up', { ...args, role:'serena', task:'more' }), /limit/)
  run.cancelled = true
  assert.throws(() => call('team_status', args), /no longer active/)
})

test('follow-ups append a new reporter and keep completed outcomes', () => {
  const { call, args, run } = fixture()
  run.delegated = true
  run.team.stages = [{ role:'nova', status:'done', iteration:1 }, { role:'nova', status:'running', iteration:1 }]
  call('team_follow_up', { ...args, role:'ralph', task:'Verify the failing check' })
  assert.deepEqual(run.team.stages.map((stage) => stage.status), ['done','running','pending','pending'])
  assert.equal(run.team.stages.at(-1)?.role, 'nova')
})
