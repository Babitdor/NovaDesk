import { AGENTS } from '../lib/settings'
import { COLUMNS, boardColumn, stamp, type Chat, type Column } from '../lib/sessions'
import type { CrewRole } from '../lib/crew'
import { AgentMark } from './AgentMark'
import { CrewPortrait } from './CrewPortrait'

const LABELS: Record<Column, string> = { todo: 'Todo', doing: 'Doing', done: 'Done' }

const EMPTY: Record<Column, string> = {
  todo: 'Nothing queued.',
  doing: 'Nothing in flight.',
  done: 'Nothing finished yet.',
}

/// The crew member a card stands for, when the task was addressed to one. "team"
/// is a mode, not a face, so it falls back to the backend's mark.
function portrait(chat: Chat): CrewRole | null {
  return chat.crew && chat.crew !== 'team' ? chat.crew : null
}

type Props = {
  chats: Chat[]
  activeId: string
  onOpen: (id: string) => void
  onMove: (id: string, column: Column) => void
}

/// The tasks as three columns. It is a projection of the same list the rail
/// shows — same chats, same order — so moving a card and renaming a task can
/// never disagree about what exists.
///
/// Cards move with the two buttons rather than a drag: a native child webview
/// paints over the page, and a drag that crosses it would be lost halfway.
export function BoardPane({ chats, activeId, onOpen, onMove }: Props) {
  return (
    <div className="board">
      {COLUMNS.map((column, index) => {
        const cards = chats.filter((chat) => boardColumn(chat) === column)
        const before = COLUMNS[index - 1]
        const after = COLUMNS[index + 1]

        return (
          <section key={column} className="board-col" aria-label={LABELS[column]}>
            <h3 className="board-head">
              <span>{LABELS[column]}</span>
              <span className="board-count">{cards.length}</span>
            </h3>

            <div className="board-list">
              {cards.map((chat) => {
                const face = portrait(chat)
                return (
                  <article key={chat.id} className={chat.id === activeId ? 'board-card is-on' : 'board-card'}>
                    <div className="board-card-head">
                      {face
                        ? <CrewPortrait role={face} />
                        : chat.agent ? <AgentMark id={chat.agent} chip /> : null}
                      <button type="button" className="board-card-title" title={chat.title}
                        onClick={() => onOpen(chat.id)}>
                        {chat.title}
                      </button>
                    </div>

                    <div className="board-card-meta">
                      <span>{chat.agent ? AGENTS[chat.agent].name : 'Unassigned'}</span>
                      <span>{chat.model ?? 'auto'}</span>
                      <span>{stamp(chat)}</span>
                    </div>

                    <div className="board-card-move">
                      <button type="button" className="ghost" disabled={!before}
                        aria-label={`Move ${chat.title} to ${before ? LABELS[before] : ''}`}
                        onClick={() => before && onMove(chat.id, before)}>
                        {'\u25c0'}
                      </button>
                      <button type="button" className="ghost" disabled={!after}
                        aria-label={`Move ${chat.title} to ${after ? LABELS[after] : ''}`}
                        onClick={() => after && onMove(chat.id, after)}>
                        {'\u25b6'}
                      </button>
                    </div>
                  </article>
                )
              })}

              {cards.length === 0 && <p className="hint">{EMPTY[column]}</p>}
            </div>
          </section>
        )
      })}
    </div>
  )
}
