import { useEffect, useState, type KeyboardEvent } from 'react'

import type { DirEntry, WorkspaceInfo } from '../lib/events'
import { searchFiles } from '../lib/ipc'
import { iconOf } from '../lib/preview'
import { FileTree } from './FileTree'
import { PreviewPanel } from './PreviewPanel'

type Props = {
  workspace: WorkspaceInfo | null
  open?: { path: string; nonce: number } | null
}

/// The tabs and the tree share one panel: a tab strip stands where the tree was,
/// so the pane keeps the width the layout gave it, and the tree is one click away
/// (close the last tab, or click a breadcrumb folder).
///
/// `open` is how a tool opens a file: the request carries a nonce so asking for
/// the path that is already shown still counts as a new request, and it lands
/// read-only — the usual reason to be handed a file is that the agent just changed
/// it. A file opened from the tree is yours to edit.
///
/// Ctrl+P opens Go to file while this pane has the focus; it is deliberately not a
/// window-wide shortcut, because the pane stays mounted while hidden.
export function FilesPane({ workspace, open }: Props) {
  const [tabs, setTabs] = useState<{ path: string; read: boolean }[]>([])
  const [request, setRequest] = useState<{ path: string; nonce: number } | null>(null)
  /// What the preview is actually showing, which is not the same as the request
  /// once a link inside a Markdown file is followed.
  const [shown, setShown] = useState<string | null>(null)
  const [view, setView] = useState<'tree' | 'file'>('tree')
  const [dirty, setDirty] = useState(false)
  const [reveal, setReveal] = useState<{ path: string; nonce: number } | null>(null)
  const [seen, setSeen] = useState(0)
  const [query, setQuery] = useState<string | null>(null)
  const [hits, setHits] = useState<DirEntry[]>([])
  const [hit, setHit] = useState(0)
  // A new request is a new nonce, so asking for the path already on screen still
  // counts as an ask. Derived from the last request rather than from a ref: this
  // runs during render too, and a ref read there is what React forbids.
  const ask = (path: string) =>
    setRequest((last) => ({ path, nonce: (last?.nonce ?? 0) + 1 }))

  const openTab = (path: string, read: boolean) => {
    setTabs((known) =>
      known.some((tab) => tab.path === path) ? known : [...known, { path, read }],
    )
    // Clicking the tab that is already on screen is not a new request: asking
    // would re-run the navigation and offer to save a clean file.
    if (path !== shown || view !== 'file') ask(path)
    setView('file')
  }

  // A file the preview followed a link to joins the strip; the request is left
  // alone, because changing it would re-run the navigation that just happened.
  const joined = (path: string) => {
    setShown(path)
    setTabs((known) => (known.some((tab) => tab.path === path) ? known : [...known, { path, read: false }]))
  }

  // Closing the tab on screen goes through the preview's own close button, which
  // is where the unsaved-changes dialog lives; a background tab holds no editor
  // (leaving one always settles its draft), so it can be dropped here.
  const closeTab = (path: string) => {
    const index = tabs.findIndex((tab) => tab.path === path)
    const rest = tabs.filter((tab) => tab.path !== path)
    setTabs(rest)
    if (path !== shown) return
    const next = rest[index] ?? rest[index - 1] ?? null
    setShown(next?.path ?? null)
    setDirty(false)
    if (next) ask(next.path)
    else {
      setRequest(null)
      setView('tree')
    }
  }

  /// Back to the tree, with a folder opened: only the preview can ask, because
  /// only it can settle an unsaved draft first.
  const showTree = (path: string) => {
    setView('tree')
    setDirty(false)
    setReveal((last) => ({ path, nonce: (last?.nonce ?? 0) + 1 }))
  }

  // Adjusting state during render, which is what React recommends for a prop
  // that should reset a view: doing it in an effect would paint the old file
  // once before the new one.
  if (open && open.nonce !== seen) {
    setSeen(open.nonce)
    openTab(open.path, true)
  }

  // One search per pause in typing, not per keystroke: the walk is bounded but it
  // is still a disk walk.
  useEffect(() => {
    // The box clears its own results on every keystroke; an effect that cleared
    // them here would be a second render for nothing.
    if (!query?.trim()) return
    let alive = true
    const timer = setTimeout(() => {
      searchFiles(query).then(
        (found) => alive && setHits(found),
        () => alive && setHits([]),
      )
    }, 120)
    return () => {
      alive = false
      clearTimeout(timer)
    }
  }, [query])

  if (!workspace) return <p className="hint">Open a folder to browse it.</p>

  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'p') {
      event.preventDefault()
      setQuery('')
      return
    }
    if (event.key === 'Escape' && query !== null) setQuery(null)
  }

  const showingFile = view === 'file' && Boolean(request)

  const take = (path: string) => {
    openTab(path, false)
    setQuery(null)
  }

  return (
    <div className="files-pane" onKeyDown={onKeyDown}>
      {tabs.length > 0 && (
        <div className="file-tabs" role="tablist" aria-label="Open files">
          {tabs.map((tab) => (
            <div key={tab.path} className={tab.path === shown ? 'file-tab is-active' : 'file-tab'}>
              <button
                type="button"
                role="tab"
                aria-selected={tab.path === shown}
                title={tab.path}
                onClick={() => openTab(tab.path, tab.read)}
              >
                <span className="tree-icon" data-icon={iconOf(tab.path)}>▪</span>
                {tab.path.slice(tab.path.lastIndexOf('/') + 1)}
                {tab.path === shown && dirty && (
                  <span className="file-tab-dirty" aria-label="Unsaved changes">●</span>
                )}
              </button>
              {tab.path !== shown && (
                <button
                  type="button"
                  className="file-tab-close"
                  aria-label={`Close ${tab.path}`}
                  onClick={() => closeTab(tab.path)}
                >
                  ✕
                </button>
              )}
            </div>
          ))}
        </div>
      )}

      {/* The tree stays mounted behind a preview so a folder the reader opened is
          still open when they come back; only the preview is dropped, which is
          where the editor and its draft live. */}
      <div className={showingFile ? 'files-body is-hidden' : 'files-body'} inert={showingFile}>
        <FileTree
          key={workspace.root}
          selected={shown}
          onSelectFile={(path) => openTab(path, false)}
          reveal={reveal}
        />
      </div>

      {showingFile && request && (
        <PreviewPanel
          path={request.path}
          root={workspace.root}
          nonce={request.nonce}
          read={tabs.find((tab) => tab.path === request.path)?.read ?? false}
          onShown={joined}
          onDirty={setDirty}
          onReveal={showTree}
          onClose={() => shown && closeTab(shown)}
        />
      )}

      {query !== null && (
        <div className="quick-open" role="dialog" aria-label="Go to file">
          <input
            autoFocus
            value={query}
            placeholder="Go to file…"
            aria-label="Go to file"
            onChange={(event) => {
              setQuery(event.target.value)
              setHit(0)
              setHits([])
            }}
            onKeyDown={(event) => {
              if (event.key === 'Escape') {
                event.preventDefault()
                return setQuery(null)
              }
              if (event.key === 'ArrowDown') {
                event.preventDefault()
                return setHit((at) => Math.min(at + 1, hits.length - 1))
              }
              if (event.key === 'ArrowUp') {
                event.preventDefault()
                return setHit((at) => Math.max(at - 1, 0))
              }
              if (event.key === 'Enter' && hits[hit]) {
                event.preventDefault()
                take(hits[hit].path)
              }
            }}
          />
          <ul className="quick-list">
            {hits.map((entry, index) => (
              <li key={entry.path}>
                <button
                  type="button"
                  className={index === hit ? 'quick-row is-selected' : 'quick-row'}
                  onMouseEnter={() => setHit(index)}
                  onClick={() => take(entry.path)}
                >
                  <span className="tree-icon" data-icon={iconOf(entry.path)}>▪</span>
                  <span className="quick-path">{entry.path}</span>
                </button>
              </li>
            ))}
          </ul>
          {query.trim() !== '' && hits.length === 0 && <p className="hint">No file matches.</p>}
        </div>
      )}
    </div>
  )
}
