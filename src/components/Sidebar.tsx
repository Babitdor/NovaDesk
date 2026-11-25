import { useState } from 'react'

import type { WorkspaceInfo } from '../lib/events'
import { railMax, railMin } from '../lib/layout'
import { Splitter } from './Splitter'
import type { Chat } from '../lib/sessions'

// Windows canonicalizes to the verbatim form (`\\?\B:\...`); the prefix is
// plumbing, so the chrome shows the path a person would write. Display only —
// every path handed back to the backend stays verbatim.
const displayPath = (root: string) => root.replace(/^\\\\\?\\UNC\\/, '\\\\').replace(/^\\\\\?\\/, '')

/// A list of tasks is scanned, not read: the stamp is the coarse distance, and
/// a live turn is the only state worth a word.
function stamp(chat: Chat): string {
  if (chat.state === 'running') return 'live'
  const minutes = Math.round((Date.now() - chat.createdAt) / 60000)
  if (minutes < 1) return 'now'
  if (minutes < 60) return `${minutes}m`
  const hours = Math.round(minutes / 60)
  return hours < 24 ? `${hours}h` : `${Math.round(hours / 24)}d`
}

type Props = {
  chats: Chat[]
  activeId: string
  workspace: WorkspaceInfo | null
  width: number
  open: boolean
  onToggle: () => void
  onSelect: (id: string) => void
  onNew: () => void
  onOpenFolder: () => void
  onSettings: () => void
  folderBusy: boolean
  onWidth: (width: number) => void
  onRename: (id: string, title: string) => void
  onPin: (id: string) => void
  onDelete: (id: string) => void
}

/// The left rail. It owns nothing: every action is a prop, so the shell stays
/// the only place that knows what a chat or a folder means.
export function Sidebar({
  chats,
  activeId,
  workspace,
  width,
  open,
  onToggle,
  onSelect,
  onNew,
  onOpenFolder,
  onSettings,
  folderBusy,
  onWidth,
  onRename,
  onPin,
  onDelete,
}: Props) {
  const [filter, setFilter] = useState('')
  /// One row menu at a time, and a rename in place: the rail is a list, so the
  /// actions belong to the row rather than to a dialog over the transcript.
  const [menuId, setMenuId] = useState<string | null>(null)
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  /// Deleting a task is the one action here that cannot be undone, so it takes
  /// two clicks on the same row.
  const [armedId, setArmedId] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const needle = filter.trim().toLowerCase()
  const sorted = [...chats].sort((a, b) => Number(Boolean(b.pinned)) - Number(Boolean(a.pinned)))
  const shown = needle ? sorted.filter((chat) => chat.title.toLowerCase().includes(needle)) : sorted

  const close = () => {
    setMenuId(null)
    setArmedId(null)
    setCopied(false)
  }

  const startRename = (chat: Chat) => {
    setMenuId(null)
    setDraft(chat.title)
    setRenamingId(chat.id)
  }

  const commitRename = () => {
    if (renamingId) onRename(renamingId, draft)
    setRenamingId(null)
  }

  const copyPath = () => {
    if (!workspace) return
    void navigator.clipboard.writeText(displayPath(workspace.root)).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1200)
    })
  }

  return (
    <aside className={open ? 'rail' : 'rail is-closed'} style={open ? { width } : undefined}>
      {open && (
        <Splitter
          axis="x"
          value={width}
          min={railMin}
          max={railMax}
          label="Resize the task list"
          onChange={onWidth}
        />
      )}
      <div className="rail-head">
        <img className="mark" src="/nova.webp" alt="" width={22} height={22} />
        <span className="wordmark">Nova Desktop</span>
        <button
          type="button"
          className="ghost collapse"
          aria-label={open ? 'Collapse task list' : 'Expand task list'}
          aria-expanded={open}
          onClick={onToggle}
        >
          <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d={open ? 'M10 3 5 8l5 5' : 'M6 3l5 5-5 5'}
              fill="none"
              stroke="currentColor"
              strokeWidth="1.6"
            />
          </svg>
        </button>
      </div>

      <button type="button" className="new-task" onClick={onNew}>
        + New task
      </button>

      <input
        className="rail-filter"
        value={filter}
        placeholder="Filter tasks"
        aria-label="Filter tasks"
        onChange={(event) => setFilter(event.target.value)}
      />

      <div className="rail-list">
        {shown.map((chat) => (
          <div key={chat.id} className="chat-item">
            {renamingId === chat.id ? (
              <input
                className="chat-rename"
                // eslint-disable-next-line jsx-a11y/no-autofocus
                autoFocus
                value={draft}
                aria-label="Task name"
                onChange={(event) => setDraft(event.target.value)}
                onBlur={commitRename}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') commitRename()
                  if (event.key === 'Escape') setRenamingId(null)
                }}
              />
            ) : (
              <button
                type="button"
                className={chat.id === activeId ? 'chat-row is-active' : 'chat-row'}
                onClick={() => onSelect(chat.id)}
                title={chat.title}
              >
                {chat.pinned && <span className="chat-pin" title="Pinned" />}
                <span className="chat-title">{chat.title}</span>
                <span className="chat-stamp">{stamp(chat)}</span>
              </button>
            )}

            <button
              type="button"
              className="chat-more"
              aria-label={`Actions for ${chat.title}`}
              aria-expanded={menuId === chat.id}
              onClick={() => {
                if (menuId === chat.id) close()
                else {
                  setMenuId(chat.id)
                  setArmedId(null)
                  setCopied(false)
                }
              }}
            >
              ⋯
            </button>

            {menuId === chat.id && (
              <>
                <div className="rail-scrim" onClick={close} />
                <div className="chat-menu" role="menu">
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => {
                      onPin(chat.id)
                      close()
                    }}
                  >
                    {chat.pinned ? 'Unpin' : 'Pin to top'}
                  </button>
                  <button type="button" role="menuitem" onClick={() => startRename(chat)}>
                    Rename
                  </button>
                  {workspace && (
                    <button type="button" role="menuitem" onClick={copyPath}>
                      {copied ? 'Copied' : 'Copy project path'}
                    </button>
                  )}
                  <button
                    type="button"
                    role="menuitem"
                    className="is-danger"
                    onClick={() => {
                      if (armedId === chat.id) {
                        onDelete(chat.id)
                        close()
                        return
                      }
                      setArmedId(chat.id)
                    }}
                  >
                    {armedId === chat.id ? 'Delete for good?' : 'Delete'}
                  </button>
                </div>
              </>
            )}
          </div>
        ))}
        {shown.length === 0 && <p className="hint">No task matches that filter.</p>}
      </div>

      <div className="rail-foot">
        <span className="root" title={workspace ? displayPath(workspace.root) : undefined}>
          {workspace ? displayPath(workspace.root) : 'No folder open'}
        </span>
        <button type="button" className="ghost" onClick={onOpenFolder} disabled={folderBusy}>
          Open folder
        </button>
      </div>
      <button type="button" className="rail-settings" onClick={onSettings} aria-label="Settings" title="Settings">
        <span aria-hidden="true">⚙</span>{open && 'Settings'}
      </button>
    </aside>
  )
}
