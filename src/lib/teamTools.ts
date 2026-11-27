import { CREW, type CrewRole, type Handoff, type TeamRun, type TeamStage } from './crew.ts'
import type { Preferences } from './settings'

export type CrewControl = { cancelled:boolean; id:string; chatId:string; team:TeamRun; notes:Handoff[]; delegated:boolean }
type Context = {
  preferences:Preferences
  run:() => CrewControl | null
  patch:(chatId:string, patch:(team:TeamRun) => TeamRun) => void
  message:(chatId:string, text:string) => void
}
export function teamTools(context:Context) {
  return (name:string, args:Record<string, unknown>) => {
    const run = context.run()
    if (!run || run.cancelled || args._runId !== run.id) throw new Error('This team run is no longer active')
    if (name === 'team_status') return { runId:run.id, assignments:run.team.stages, handoffs:run.team.handoffs }
    const sender = args._sender as CrewRole
    if (!Object.hasOwn(CREW, sender)) throw new Error('Unknown teammate')
    if (name === 'team_message') {
      const to = args.to as CrewRole
      if (!Object.hasOwn(CREW, to) || typeof args.text !== 'string' || !args.text.trim() || args.text.length > 8000) throw new Error('Invalid addressed team message')
      const note: Handoff = { from:sender, to, harness:String(args._harness ?? ''), text:args.text.trim() }
      run.notes.push(note)
      context.patch(run.chatId, (team) => ({ ...team, handoffs:[...team.handoffs, note] }))
      context.message(run.chatId, CREW[sender].name + ' → ' + CREW[to].name + ' · ' + note.harness + ': ' + note.text)
      return { delivered:true, to, timing:'Available to the recipient next turn' }
    }
    if (name === 'team_delegate' || name === 'team_follow_up') {
      const role = args.role as CrewRole
      if (sender !== 'nova' || !['nora','serena','ralph'].includes(role) || typeof args.task !== 'string' || !args.task.trim() || args.task.length > 12000) throw new Error('Invalid team assignment')
      if (context.preferences.team.roles[role] === false) throw new Error('This team member is disabled')
      const harness = context.preferences.crew[role]?.agent ?? 'nova'
      const assignment: TeamStage = { role, harness, model:context.preferences.crew[role]?.model, systemPrompt:context.preferences.crew[role]?.systemPrompt, task:args.task.trim(), assignmentId:crypto.randomUUID(),
        readOnly:args.investigate === true && harness === 'codex', status:'pending', iteration:1 }
      let stages = run.team.stages
      if (!run.delegated) stages = stages.filter((stage, at) => at === 0 || stage.status !== 'pending' || at === stages.length - 1)
      if (stages.length >= 100 || stages.filter((stage) => stage.status === 'pending' && stage.role !== 'nova').length >= context.preferences.agentLimit) throw new Error('Team assignment limit reached')
      run.delegated = true
      const reporter = stages.at(-1)
      stages = reporter?.role === 'nova' && reporter.status === 'pending'
        ? [...stages.slice(0,-1), assignment, reporter]
        : [...stages, assignment, { role:'nova', harness:context.preferences.crew.nova?.agent ?? 'nova', status:'pending', iteration:1, assignmentId:crypto.randomUUID() }]
      context.patch(run.chatId, (team) => ({ ...team, stages }))
      return { assignmentId:assignment.assignmentId, role, harness, readOnly:assignment.readOnly,
        status:'queued', execution:assignment.readOnly ? 'Read-only investigation' : 'Serialized; exclusive editing turn' }
    }
    throw new Error('Unknown team operation')
  }
}
