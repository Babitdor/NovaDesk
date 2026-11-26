import { useCallback, useEffect, useState } from 'react'

import type { DiffLine } from '../lib/diff'
import type { WorkspaceInfo } from '../lib/events'
import { readTree, type TreeSnapshot } from '../lib/git'

/// The working tree as a grouped diff: one row per file with its own counts, and
/// the hunks underneath — which is how a change is actually read.
///
/// `refreshKey` is for a host that already owns a Refresh: the GitHub pane bumps
/// it so one click updates its header and this diff together, and the diff stays
/// on screen while the read is in flight.
export function ChangesPane({ workspace, refreshKey = 0 }: { workspace: WorkspaceInfo | null; refreshKey?: number }) {
  const [snapshot, setSnapshot] = useState<TreeSnapshot | null>(null)
  const [openPath, setOpenPath] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const files = snapshot && 'files' in snapshot ? snapshot.files : null
  const error = snapshot && 'error' in snapshot ? snapshot.error : null
  const added = files?.reduce((total, file) => total + file.added, 0) ?? 0
  const removed = files?.reduce((total, file) => total + file.removed, 0) ?? 0

  const refresh = useCallback(async () => {
    if (!workspace) return
    setBusy(true)
    const next = await readTree()
    setSnapshot(next)
    // The first file opens itself: a list of collapsed groups is a table of
    // contents, not a diff.
    setOpenPath('files' in next && next.files.length > 0 ? next.files[0].path : null)
    setBusy(false)
  }, [workspace])

  // The pane is a view of a folder that changes while it is open, so the read
  // belongs to the effect; the state it lands in is the repository's. The host's
  // refresh is a second trigger for the same read.
  useEffect(() => {
    if (!workspace) return
    let alive = true
    void readTree().then((next) => {
      if (!alive) return
      setSnapshot(next)
      setOpenPath('files' in next && next.files.length > 0 ? next.files[0].path : null)
    })
    return () => {
      alive = false
    }
  }, [refreshKey, workspace])

  return (
    <section className="changes">
      <header className="pane-head">
        <span>
          {!workspace
            ? 'no folder open'
            : snapshot === null
              ? 'reading…'
              : error
                ? 'no repository'
                : (files?.length ?? 0) + ' files'}
        </span>
        <button
          type="button"
          className="ghost"
          onClick={() => void refresh()}
          disabled={busy || !workspace}
        >
          Refresh
        </button>
      </header>

      {error && <p className="panel-error">{error}</p>}

      {files && (
        <div className="diff-summary">
          <span className="diff-scope">All changes</span>
          <span className="diff-total">
            <span className="stat-add">{'+' + added}</span>
            <span className="stat-del">{'−' + removed}</span>
          </span>
          {snapshot && 'branch' in snapshot && snapshot.branch && (
            <span className="diff-branch">{snapshot.branch}</span>
          )}
        </div>
      )}

      {files && files.length === 0 && <p className="hint">Nothing changed in this folder.</p>}

      <div className="file-list">
        {(files ?? []).map((file) => {
          const slash = file.path.lastIndexOf('/')
          const name = slash === -1 ? file.path : file.path.slice(slash + 1)
          const dir = slash === -1 ? '' : file.path.slice(0, slash + 1)
          const expanded = openPath === file.path

          return (
            <div key={file.path} className="file-group">
              <button
                type="button"
                className={expanded ? 'file-row is-open' : 'file-row'}
                aria-expanded={expanded}
                onClick={() => setOpenPath(expanded ? null : file.path)}
              >
                <svg className="file-chevron" width="10" height="10" viewBox="0 0 16 16" aria-hidden="true">
                  <path
                    d={expanded ? 'M3 6l5 5 5-5' : 'M6 3l5 5-5 5'}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.8"
                  />
                </svg>
                <span className="file-name">{name}</span>
                <span className="file-dir">{dir}</span>
                <span className="diff-total">
                  {file.added > 0 && <span className="stat-add">{'+' + file.added}</span>}
                  {file.removed > 0 && <span className="stat-del">{'−' + file.removed}</span>}
                </span>
              </button>

              {expanded &&
                (file.lines.length > 0 ? (
                  <DiffBody lines={file.lines} />
                ) : (
                  <p className="hint">No diff yet — the file is untracked, binary or unchanged.</p>
                ))}
            </div>
          )
        })}
      </div>
    </section>
  )
}

/// Two gutters and the code: the numbers are what let a reader jump from the pane
/// to the file, so they are part of the line, not a decoration.
function DiffBody({ lines }: { lines: DiffLine[] }) {
  return (
    <div className="diff-body">
      {lines.map((line, index) =>
        line.kind === 'hunk' ? (
          <div key={index} className="diff-line is-hunk">
            {line.text}
          </div>
        ) : (
          <div key={index} className={'diff-line is-' + line.kind}>
            <span className="diff-gutter">{line.oldNo ?? ''}</span>
            <span className="diff-gutter">{line.newNo ?? ''}</span>
            <span className="diff-code">
              {(line.kind === 'add' ? '+' : line.kind === 'del' ? '-' : ' ') + line.text}
            </span>
          </div>
        ),
      )}
    </div>
  )
}
