import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react'

import type { DirEntry } from '../lib/events'
import { errorText, listDir } from '../lib/ipc'
import { iconOf } from '../lib/preview'

type Props = {
  selected: string | null
  onSelectFile: (path: string) => void
  /// A folder the pane wants opened, from a breadcrumb: expanding the path to it
  /// is what makes the crumbs navigation rather than a label.
  reveal?: { path: string; nonce: number } | null
}

type Row = DirEntry & { depth: number }

/// The tree is one flat list of rows, not nested lists: every tree key (up, down,
/// left, right) needs the row before and after this one, and depth is drawn with
/// padding. That is also what lets the row under the cursor be scrolled to.
///
/// Folders load once, on first expand, so opening a big repo costs one `list_dir`
/// per level the user actually looked at.
export function FileTree({ selected, onSelectFile, reveal }: Props) {
  const [dirs, setDirs] = useState<Record<string, DirEntry[]>>({})
  const [open, setOpen] = useState<Record<string, boolean>>({})
  const [cursor, setCursor] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const loading = useRef(new Set<string>())
  const cursorRow = useRef<HTMLButtonElement>(null)

  const load = useCallback((path: string) => {
    if (loading.current.has(path)) return
    loading.current.add(path)
    listDir(path).then(
      (entries) => setDirs((known) => ({ ...known, [path]: entries })),
      (err) => {
        loading.current.delete(path)
        setError(errorText(err))
      },
    )
  }, [])

  // The root is the one folder opened for free; everything else waits for a click.
  useEffect(() => {
    let alive = true
    listDir('').then(
      (entries) => alive && setDirs((known) => ({ ...known, '': entries })),
      (err) => alive && setError(errorText(err)),
    )
    return () => {
      alive = false
    }
  }, [])

  const revealed = useRef(0)
  useEffect(() => {
    if (!reveal || reveal.nonce === revealed.current) return
    revealed.current = reveal.nonce
    const parts = reveal.path.split('/').filter(Boolean)
    const folders = parts.map((_, index) => parts.slice(0, index + 1).join('/'))
    for (const folder of folders) load(folder)
    setOpen((known) => ({
      ...known,
      ...Object.fromEntries(folders.map((folder) => [folder, true])),
    }))
    setCursor(reveal.path)
  }, [reveal, load])

  useEffect(() => {
    cursorRow.current?.scrollIntoView({ block: 'nearest' })
  }, [cursor])

  // Rows are derived, not stored: one walk over the folders that are open is
  // cheaper to keep right than a list that has to be updated on every load.
  const rows: Row[] = []
  const walk = (path: string, depth: number) => {
    for (const entry of dirs[path] ?? []) {
      rows.push({ ...entry, depth })
      if (entry.isDir && open[entry.path]) walk(entry.path, depth + 1)
    }
  }
  walk('', 0)

  const at = rows.findIndex((row) => row.path === cursor)
  const step = (delta: number) => {
    if (rows.length === 0) return
    setCursor(rows[Math.min(Math.max(at + delta, 0), rows.length - 1)].path)
  }

  const activate = (row: Row) => {
    if (!row.isDir) {
      setCursor(row.path)
      onSelectFile(row.path)
      return
    }
    const opening = !open[row.path]
    if (opening) load(row.path)
    setOpen((known) => ({ ...known, [row.path]: opening }))
    setCursor(row.path)
  }

  // VS Code's tree keys: up and down move, right opens or enters, left closes or
  // goes to the parent, Enter acts. Prevented entries never reach the button's
  // own Enter handling, which would fire the click a second time.
  const onKeyDown = (event: KeyboardEvent) => {
    const row = rows[at]
    const keys: Record<string, () => void> = {
      ArrowDown: () => step(1),
      ArrowUp: () => step(-1),
      ArrowRight: () => {
        if (!row) return step(0)
        if (row.isDir && !open[row.path]) return activate(row)
        const child = rows[at + 1]
        if (child?.depth === row.depth + 1) setCursor(child.path)
      },
      ArrowLeft: () => {
        if (!row) return step(0)
        if (row.isDir && open[row.path]) {
          setOpen((known) => ({ ...known, [row.path]: false }))
          return setCursor(row.path)
        }
        const cut = row.path.lastIndexOf('/')
        setCursor(cut === -1 ? '' : row.path.slice(0, cut))
      },
      Enter: () => {
        if (row) activate(row)
      },
    }
    const run = keys[event.key]
    if (!run) return
    event.preventDefault()
    run()
  }

  return (
    <div className="tree" role="tree" aria-label="Workspace files" onKeyDown={onKeyDown}>
      {error && <p className="tree-error">{error}</p>}
      {!dirs[''] && !error && <p className="tree-loading">Loading…</p>}
      <div className="tree-list">
        {rows.map((row) => {
          const isCursor = row.path === cursor
          return (
            <button
              key={row.path}
              type="button"
              role="treeitem"
              aria-level={row.depth + 1}
              aria-expanded={row.isDir ? !!open[row.path] : undefined}
              aria-selected={row.path === selected}
              tabIndex={isCursor ? 0 : -1}
              ref={isCursor ? cursorRow : undefined}
              className={row.path === selected ? 'tree-row is-selected' : 'tree-row'}
              style={{ paddingLeft: `${8 + row.depth * 14}px` }}
              onClick={() => activate(row)}
            >
              <span className="tree-icon" data-icon={row.isDir ? undefined : iconOf(row.path)}>
                {row.isDir ? (open[row.path] ? '▾' : '▸') : '▪'}
              </span>
              <span className="tree-name">{row.name}</span>
            </button>
          )
        })}
      </div>
    </div>
  )
}
