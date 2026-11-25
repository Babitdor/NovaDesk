import { useCallback, useEffect, useState } from 'react'

import { parseDiff, type DiffLine, type FileDiff } from '../lib/diff'
import type { WorkspaceInfo } from '../lib/events'
import { runCommand } from '../lib/ipc'

type Captured = { ok: boolean; out: string; err: string }

/// `run_command` streams stdout down a channel and reports only an exit code on
/// completion, so a caller that needs the text has to collect it here.
async function capture(program: string, args: string[]): Promise<Captured> {
  const commandId = crypto.randomUUID()
  let out = ''
  let err = ''

  const settled = new Promise<boolean>((resolve) => {
    void runCommand(commandId, program, args, (event) => {
      if (event.type === 'processOutput') {
        if (event.stream === 'stdout') out += event.chunk
        else err += event.chunk
      } else if (event.type === 'toolResult' && event.callId === commandId) {
        resolve(event.ok)
      }
    }).catch(() => resolve(false))
  })

  return { ok: await settled, out, err }
}

type Snapshot = { files: FileDiff[]; branch: string } | { error: string }

/// Reads the whole working tree through the same process runner the terminal
/// uses, so the pane needs no new backend command and can never see outside the
/// workspace. It returns the state instead of setting it, so the mount and the
/// Refresh button share one implementation.
async function readTree(): Promise<Snapshot> {
  // `--is-inside-work-tree` answers with or without a first commit, which
  // `rev-parse HEAD` does not.
  const inside = await capture('git', ['rev-parse', '--is-inside-work-tree'])
  if (!inside.ok) return { error: 'Not a git repository.' }

  const branch = await capture('git', ['rev-parse', '--abbrev-ref', 'HEAD'])
  const diff = await capture('git', ['diff', 'HEAD'])
  const status = await capture('git', ['status', '--porcelain=v1'])

  // An untracked file has no diff until it is staged, so it is listed from the
  // status rather than invented.
  const untracked = status.ok
    ? status.out
        .split('\n')
        .filter((line) => line.startsWith('?? '))
        .map((line) => line.slice(3).trim())
    : []

  return {
    files: [
      ...(diff.ok ? parseDiff(diff.out) : []),
      ...untracked.map((path): FileDiff => ({ path, added: 0, removed: 0, lines: [] })),
    ],
    branch: branch.ok ? branch.out.trim() : '',
  }
}

/// The working tree as a grouped diff: one row per file with its own counts, and
/// the hunks underneath — which is how a change is actually read.
export function ChangesPane({ workspace }: { workspace: WorkspaceInfo | null }) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null)
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
  // belongs to the effect; the state it lands in is the repository's.
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
  }, [workspace])

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
