import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'

import type { ChatItem } from '../lib/chat'
import { CREW, crewRoles, ralphRetry } from '../lib/crew'
import { CrewPortrait } from './CrewPortrait'
import { Markdown } from './Markdown'
import { Playback } from './Playback'
import { NovaCoin } from './NovaCoin'

const FOLLOW_DISTANCE = 32

const ConversationItem = memo(function ConversationItem({ item, markdown, backend, expand }: {
  item: ChatItem; markdown: boolean; backend: string; expand: boolean
}) {
  const [copy, setCopy] = useState('')
  switch (item.kind) {
    case 'user':
      return <div className="msg msg-user">
        {item.recipient && <small className="message-recipient">@{item.recipient === 'team' ? 'Team' : CREW[item.recipient].name}</small>}
        {item.text}
      </div>
    case 'assistant': {
      const author = item.author ?? backend
      const role = item.speaker?.role ?? crewRoles.find((entry) => CREW[entry].name.toLowerCase() === author.toLowerCase())
      const name = role ? CREW[role].name : author
      const text = role === 'ralph' ? item.text.replaceAll(ralphRetry, '').trimEnd() : item.text
      return (
        <article className="msg msg-assistant" aria-label={name} data-stage={item.speaker?.stage}>
          <header className="reply-head">
            <div className="reply-speaker">
              {role && <CrewPortrait role={role} />}
              <div>
                <strong>{name}</strong>
                {role && <small>{item.speaker?.to ? 'To ' + CREW[item.speaker.to].name : CREW[role].title}{item.speaker?.harness && ' · ' + item.speaker.harness}</small>}
              </div>
            </div>
            {!item.provisional && text && <button className="ghost reply-copy" type="button" onClick={() => {
              if (!navigator.clipboard) {
                setCopy('Copy unavailable')
                return
              }
              void navigator.clipboard.writeText(text).then(() => setCopy('Copied'), () => setCopy('Copy failed'))
            }}>{copy || 'Copy reply'}</button>}
            {!item.provisional && !item.plain && text && <Playback id={item.id} text={text} />}
          </header>
          {markdown && !item.plain ? <Markdown text={text} /> : <div className="reply-plain">{text}</div>}
        </article>
      )
    }
    case 'notice':
      return <div className={`notice-line notice-${item.level}`}>{item.text}</div>
    case 'error':
      return <div className="msg msg-error">{item.text}</div>
    case 'tool':
      return (
        <details className="msg msg-tool" open={expand}>
          <summary>
            <span className="tool-name">{item.label}</span>
            <span className={item.ok === undefined ? 'badge' : item.ok ? 'badge ok' : 'badge bad'}>
              {item.ok === undefined ? 'Running' : item.ok ? 'Done' : 'Failed'}
            </span>
          </summary>
          {item.input && <pre className="tool-output">{item.input}</pre>}
          {item.stream && <pre className="tool-output">{item.stream}</pre>}
          {item.output && <pre className="tool-output">{item.output}</pre>}
        </details>
      )
  }
})

export function ChatLog({ items, busy, markdown, backend, workspaceReady, compact, expandTools }: {
  items: ChatItem[]; busy: boolean; markdown: boolean; backend: string; workspaceReady: boolean
  compact: boolean; expandTools: boolean
}) {
  const viewport = useRef<HTMLDivElement>(null)
  const flow = useRef<HTMLDivElement>(null)
  const following = useRef(true)
  const [showLatest, setShowLatest] = useState(false)
  const [handoff, setHandoff] = useState(true)

  useEffect(() => {
    // Existing transcript entries skip the entrance animation on a task switch.
    const frame = requestAnimationFrame(() => setHandoff(false))
    return () => cancelAnimationFrame(frame)
  }, [])
  const scrollToLatest = useCallback(() => {
    if (viewport.current) viewport.current.scrollTop = viewport.current.scrollHeight
  }, [])

  useLayoutEffect(() => {
    if (following.current) scrollToLatest()
  }, [items, busy, scrollToLatest])

  useLayoutEffect(() => {
    const observer = new ResizeObserver(() => {
      if (following.current) scrollToLatest()
    })
    if (flow.current) observer.observe(flow.current)
    if (viewport.current) observer.observe(viewport.current)
    return () => observer.disconnect()
  }, [scrollToLatest])

  return (
    <div className="chat-area">
      <div className="chat-log" ref={viewport} role="region" aria-label="Conversation" tabIndex={0}
        data-compact={compact ? 'true' : undefined}
        onScroll={(event) => {
          const el = event.currentTarget
          following.current = el.scrollHeight - el.clientHeight - el.scrollTop <= FOLLOW_DISTANCE
          setShowLatest(!following.current)
        }}>
        <div className={handoff ? 'chat-flow is-handoff' : 'chat-flow'} ref={flow}>
          {items.length === 0 && (
            <div className="empty">
              <NovaCoin />
              <h2>Nova Desktop</h2>
              {!workspaceReady && <p>Open a workspace to start a task.</p>}
            </div>
          )}
          {items.map((item) => <ConversationItem key={item.id} item={item} markdown={markdown}
            backend={backend} expand={expandTools} />)}
        </div>
      </div>
      {showLatest && <button type="button" className="jump-to-latest" onClick={() => {
        following.current = true
        viewport.current?.focus({ preventScroll: true })
        scrollToLatest()
        setShowLatest(false)
      }}>↓ Jump to latest</button>}
    </div>
  )
}
