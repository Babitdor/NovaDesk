import { useEffect, useState } from 'react'

import type { DirEntry } from '../lib/events'
import { errorText, listDir } from '../lib/ipc'

type Props = {
  selected: string | null
  onSelectFile: (path: string) => void
}

/// Loads one directory level and recurses only into expanded folders, so opening
/// a big repo costs one `list_dir` per level the user actually looks at.
export function FileTree({ selected, onSelectFile }: Props) {
  return (
    <div className="tree">
      <TreeBranch path="" depth={0} selected={selected} onSelectFile={onSelectFile} />
    </div>
  )
}

type BranchProps = Props & { path: string; depth: number }

function TreeBranch({ path, depth, selected, onSelectFile }: BranchProps) {
  const [entries, setEntries] = useState<DirEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [open, setOpen] = useState<Record<string, boolean>>({})

  // One fetch per mount: a branch's path never changes while it is mounted.
  useEffect(() => {
    let alive = true
    listDir(path)
      .then((result) => alive && setEntries(result))
      .catch((err) => alive && setError(errorText(err)))
    return () => {
      alive = false
    }
  }, [path])

  if (error) return <p className="tree-error">{error}</p>
  if (!entries) return <p className="tree-loading">Loading…</p>

  return (
    <ul className="tree-list">
      {entries.map((entry) => (
        <li key={entry.path}>
          <button
            type="button"
            className={entry.path === selected ? 'tree-row is-selected' : 'tree-row'}
            style={{ paddingLeft: `${8 + depth * 14}px` }}
            onClick={() =>
              entry.isDir
                ? setOpen((prev) => ({ ...prev, [entry.path]: !prev[entry.path] }))
                : onSelectFile(entry.path)
            }
          >
            <span className="tree-icon">{entry.isDir ? (open[entry.path] ? '▾' : '▸') : '·'}</span>
            <span className="tree-name">{entry.name}</span>
          </button>
          {entry.isDir && open[entry.path] && (
            <TreeBranch
              path={entry.path}
              depth={depth + 1}
              selected={selected}
              onSelectFile={onSelectFile}
            />
          )}
        </li>
      ))}
    </ul>
  )
}
