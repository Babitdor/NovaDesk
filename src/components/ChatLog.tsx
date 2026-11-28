import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'

import { ActivityGroup, Thinking } from './ActivityGroup'
import { activityEntries } from '../lib/activity'
import type { ChatItem } from '../lib/chat'
import { CREW, crewRoles, ralphRetry } from '../lib/crew'
import { CrewPortrait } from './CrewPortrait'
import { Markdown } from './Markdown'
import { Playback } from './Playback'
import { NovaCoin } from './NovaCoin'

const FOLLOW_DISTANCE = 32

/// Two sheets, one behind the other: the reply can be copied, and a check says it
/// was without spending a line of text on the buttons.
function CopyGlyph() {
  return <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
    <rect x="5.8" y="2.6" width="7.6" height="9" rx="1.2" fill="none" stroke="currentColor" strokeWidth="1.3" />
    <path d="M10.2 12.4v.5a1.2 1.2 0 0 1-1.2 1.2H4a1.2 1.2 0 0 1-1.2-1.2V6.4A1.2 1.2 0 0 1 4 5.2h.5"
      fill="none" stroke="currentColor" strokeWidth="1.3" />
  </svg>
}

function CheckGlyph() {
  return <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
    <path d="M3.4 8.4 6.6 11.6 12.6 4.6" fill="none" stroke="currentColor" strokeWidth="1.6"
      strokeLinecap="round" strokeLinejoin="round" />
  </svg>
}

/// One node branching off another: this task continues from this reply.
function ForkGlyph() {
  return <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
    <path d="M4.6 11.6V7.4a2 2 0 0 1 2-2h4.1" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
    <path d="M9.2 3.6 11.4 5.4 9.2 7.2" fill="none" stroke="currentColor" strokeWidth="1.3"
      strokeLinecap="round" strokeLinejoin="round" />
    <circle cx="4.6" cy="3.4" r="1.7" fill="none" stroke="currentColor" strokeWidth="1.3" />
    <circle cx="4.6" cy="12.6" r="1.7" fill="none" stroke="currentColor" strokeWidth="1.3" />
  </svg>
}

const ConversationItem = memo(function ConversationItem({ item, markdown, backend, onFork }: {
  item: ChatItem; markdown: boolean; backend: string; onFork: (itemId: string) => void
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
            {!item.provisional && text && (
              <div className="reply-actions">
                <button className="ghost" type="button" title={copy || 'Copy reply'} aria-label={copy || 'Copy reply'}
                  onClick={() => {
                    if (!navigator.clipboard) {
                      setCopy('Copy unavailable')
                      return
                    }
                    void navigator.clipboard.writeText(text).then(() => setCopy('Copied'), () => setCopy('Copy failed'))
                  }}>
                  {copy === 'Copied' ? <CheckGlyph /> : <CopyGlyph />}
                </button>
                {copy && copy !== 'Copied' && <small role="status">{copy}</small>}
                {!item.plain && <Playback id={item.id} text={text} />}
                <button className="ghost" type="button" title="Fork from this reply"
                  aria-label="Fork from this reply" onClick={() => onFork(item.id)}>
                  <ForkGlyph />
                </button>
              </div>
            )}
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
    case 'thinking':
      return null
  }
})

export function ChatLog({ items, busy, markdown, backend, workspaceReady, compact, expandTools, onFork }: {
  items: ChatItem[]; busy: boolean; markdown: boolean; backend: string; workspaceReady: boolean
  compact: boolean; expandTools: boolean; onFork: (itemId: string) => void
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

  const latestTurn = items.slice(items.findLastIndex((item) => item.kind === 'user') + 1)
  const waitingForProgress = busy && !latestTurn.some((item) => item.kind === 'thinking' || item.kind === 'tool' || item.kind === 'assistant')

  return (
    <div className="chat-area">
      <div className="chat-log" ref={viewport} role="region" aria-label="Conversation" tabIndex={0}
        data-compact={compact ? 'true' : undefined}
        onClickCapture={(event) => {
          // Inspecting a disclosure must not scroll it away to follow the reply.
          if ((event.target as HTMLElement).closest('summary')) {
            following.current = false
            setShowLatest(true)
          }
        }}
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
          {activityEntries(items).map((item) => item.kind === 'activity'
            ? <ActivityGroup key={item.id} items={item.items} expand={expandTools} />
            : <ConversationItem key={item.id} item={item} markdown={markdown} backend={backend} onFork={onFork} />)}
          {waitingForProgress && <Thinking text="" active /> }
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
