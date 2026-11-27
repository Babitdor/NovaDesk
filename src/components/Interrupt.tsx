import { useId, useState } from 'react'
import type { AgentEvent, InterruptReply } from '../lib/events'
import { Markdown } from './Markdown'

type Request = Extract<AgentEvent, { type: 'approvalRequired' }>

export function Interrupt({ request, onReply, pending }: {
  request: Request; onReply: (reply: InterruptReply) => void; pending: boolean
}) {
  const [answer, setAnswer] = useState('')
  const [selected, setSelected] = useState<number | null>(null)
  const title = useId()
  const payload = request.payload && typeof request.payload === 'object'
    ? request.payload as Record<string, unknown> : {}
  if (request.kind === 'question') {
    if (Array.isArray(payload.questions) && payload.questions.length) {
      return <BatchQuestions questions={payload.questions} pending={pending} onReply={onReply} />
    }
    const options = Array.isArray(payload.options) ? payload.options : []
    const response = selected === null ? answer.trim() : String(options[selected])
    return (
      <form className="approval approval-input" aria-labelledby={title} onSubmit={(event) => {
        event.preventDefault()
        if (response && !pending) onReply({ kind: 'question', answer: response, selectedIndex: selected })
      }}>
        <h2 id={title}>{String(payload.question ?? 'Nova has a question')}</h2>
        {options.length > 0 && <div className="approval-options" role="group" aria-label="Suggested answers">
          {options.map((option, index) => typeof option === 'string' && (
            <label className="approval-option" key={index}>
              <input type="radio" name="suggestion" checked={selected === index} disabled={pending}
                onChange={() => { setSelected(index); setAnswer('') }} />
              <span>{option}</span>
            </label>
          ))}
        </div>}
        <label className="approval-field">{options.length ? 'Custom answer' : 'Answer'}
          <input name="answer" value={answer} maxLength={8000} disabled={pending}
            onChange={(event) => { setAnswer(event.target.value); setSelected(null) }} />
        </label>
        <div className="approval-actions">
          <button type="submit" className="approve" disabled={pending || !response}>Reply</button>
          <button type="button" disabled={pending}
            onClick={() => onReply({ kind: 'question', answer: '', selectedIndex: null })}>Skip</button>
        </div>
      </form>
    )
  }
  const plan = request.kind === 'plan'
  return (
    <section className="approval approval-input" aria-labelledby={title}>
      <h2 id={title}>{plan ? 'Review Nova’s plan' : 'Nova needs permission'}</h2>
      {plan && typeof payload.plan === 'string'
        ? <div className="approval-detail"><Markdown text={payload.plan} /></div>
        : <pre className="approval-detail">{JSON.stringify(request.payload, null, 2)}</pre>}
      {plan && <label className="approval-field">Feedback
        <input value={answer} maxLength={8000} disabled={pending} onChange={(event) => setAnswer(event.target.value)} />
      </label>}
      <div className="approval-actions">
        <button type="button" className="approve" disabled={pending} onClick={() =>
          onReply(plan ? { kind: 'plan', approve: true } : { kind: 'tool', approve: true })}>
          {plan ? 'Approve plan' : 'Allow'}
        </button>
        <button type="button" className="deny" disabled={pending} onClick={() =>
          onReply(plan ? { kind: 'plan', approve: false, feedback: answer.trim() } : { kind: 'tool', approve: false })}>
          {plan ? 'Request changes' : 'Deny'}
        </button>
      </div>
    </section>
  )
}

function BatchQuestions({ questions, pending, onReply }: { questions: unknown[]; pending:boolean; onReply:(reply:InterruptReply) => void }) {
  const [answers, setAnswers] = useState<Record<string, string>>({})
  const fields = questions.flatMap((value) => {
    if (!value || typeof value !== 'object') return []
    const question = value as Record<string, unknown>
    if (typeof question.question !== 'string') return []
    const key = typeof question.id === 'string' ? question.id : question.question
    return [{ key, text:question.question, options:Array.isArray(question.options) ? question.options : [] }]
  })
  return <form className="approval approval-input" aria-label="Teammate questions" onSubmit={(event) => {
    event.preventDefault()
    if (!pending) onReply({ kind:'question', answer:JSON.stringify(answers), selectedIndex:null })
  }}>
    {fields.map((field) => <fieldset key={field.key} disabled={pending}>
      <legend>{field.text}</legend>
      {field.options.map((value, index) => {
        const label = typeof value === 'string' ? value : value && typeof value === 'object' && 'label' in value ? String(value.label) : ''
        return label && <button type="button" key={index} aria-pressed={answers[field.key] === label}
          onClick={() => setAnswers((previous) => ({ ...previous, [field.key]:label }))}>{label}</button>
      })}
      <label>Answer<input maxLength={8000} value={answers[field.key] ?? ''}
        onChange={(event) => setAnswers((previous) => ({ ...previous, [field.key]:event.target.value }))} /></label>
    </fieldset>)}
    <div className="approval-actions">
      <button type="submit" disabled={pending || !fields.length || fields.some((field) => !answers[field.key]?.trim())}>Reply</button>
      <button type="button" disabled={pending} onClick={() => onReply({ kind:'question', answer:'{}', selectedIndex:null })}>Skip</button>
    </div>
  </form>
}
