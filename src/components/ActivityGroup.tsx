import { useLayoutEffect, useRef, type ComponentPropsWithoutRef } from 'react'
import { activityCounts, toolLabel, type ActivityItem } from '../lib/activity'
import { CREW } from '../lib/crew'

// The browser keeps disclosure state while streaming updates rerender its parent.
function ActivityDisclosure({ expand, ...props }: Omit<ComponentPropsWithoutRef<'details'>, 'open'> & { expand:boolean }) {
  const ref = useRef<HTMLDetailsElement>(null)
  useLayoutEffect(() => { if (ref.current) ref.current.open = expand }, [expand])
  return <details {...props} ref={ref} />
}

export function Thinking({ text, active, author }: { text:string; active:boolean; author?:string }) {
  return <details className="thinking-panel">
    <summary><span className={active ? 'activity-dot is-live' : 'activity-dot'} aria-hidden="true" /><span>{author ? author + ' · ' : ''}{active ? 'Thinking…' : 'Thinking'}</span></summary>
    <div className="thinking-text">{text || 'Waiting for the harness. Thinking text will appear here if it is provided.'}</div>
  </details>
}

export function ActivityGroup({ items, expand }: { items:ActivityItem[]; expand:boolean }) {
  const counts = activityCounts(items)
  const running = counts.running > 0 || counts.thinking
  const latest = items.findLast((item) => item.kind === 'tool' && item.ok === undefined && !item.finished)
  return <ActivityDisclosure className="activity-group" expand={expand}>
    <summary className="activity-summary">
      <span className={running ? 'activity-dot is-live' : 'activity-dot'} aria-hidden="true" />
      <span className="activity-title">{running ? 'Working' : 'Activity'}{counts.tools > 0 && ` · ${counts.tools} tool${counts.tools === 1 ? '' : 's'}`}</span>
      {counts.failed > 0 && <span className="activity-failed">{counts.failed} failed</span>}
      {counts.interrupted > 0 && <span className="activity-muted">{counts.interrupted} interrupted</span>}
      {latest?.kind === 'tool' && <span className="activity-current" title={latest.label}>{toolLabel(latest.label)}</span>}
    </summary>
    <div className="activity-items">
      {items.map((item) => item.kind === 'thinking'
        ? <Thinking key={item.id} text={item.text} active={item.active} author={item.speaker && CREW[item.speaker.role].name} />
        : <ActivityDisclosure key={item.id} className="tool-row" expand={expand}>
          <summary>
            <span className="tool-row-name" title={item.label}>{toolLabel(item.label)}</span>
            <span className={item.ok === false ? 'tool-state is-failed' : 'tool-state'}>{item.ok === undefined ? item.finished ? 'Interrupted' : 'Running' : item.ok ? 'Done' : 'Failed'}</span>
          </summary>
          <div className="tool-row-detail">
            {item.input && <div><small>Input</small><pre className="tool-output">{item.input}</pre></div>}
            {item.stream && <div><small>Live output</small><pre className="tool-output">{item.stream}</pre></div>}
            {item.output && <div><small>Output</small><pre className="tool-output">{item.output}</pre></div>}
          </div>
        </ActivityDisclosure>)}
    </div>
  </ActivityDisclosure>
}
