import { parseDiff, type FileDiff } from './diff'
import { runCommand } from './ipc'

export type Captured = { ok: boolean; out: string; err: string }

/// `run_command` streams stdout down a channel and reports only an exit code on
/// completion, so a caller that needs the text has to collect it here.
export async function capture(program: string, args: string[]): Promise<Captured> {
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

/// One runner for every read the git panes make: they say what they want to
/// know, not how to ask for it. Not a failure `ok: false` — a workspace that is
/// not a repository answers that way, and the pane explains it instead.
export const git = (...args: string[]) => capture('git', args)

export type TreeSnapshot = { files: FileDiff[]; branch: string } | { error: string }

/// Reads the whole working tree through the same process runner the terminal
/// uses, so the panes need no new backend command and can never see outside the
/// workspace. It returns the state instead of setting it, so a mount and a
/// Refresh button share one implementation.
export async function readTree(): Promise<TreeSnapshot> {
  // `--is-inside-work-tree` answers with or without a first commit, which
  // `rev-parse HEAD` does not.
  const inside = await git('rev-parse', '--is-inside-work-tree')
  if (!inside.ok) return { error: 'Not a git repository.' }

  const branch = await git('rev-parse', '--abbrev-ref', 'HEAD')
  const diff = await git('diff', 'HEAD')
  const status = await git('status', '--porcelain=v1')

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
