/// One line of a unified diff. Both line numbers are kept, because the gutter is
/// what makes a diff readable — the signs alone leave you counting.
export type DiffLine = {
  kind: 'add' | 'del' | 'ctx' | 'hunk'
  oldNo: number | null
  newNo: number | null
  text: string
}

export type FileDiff = {
  path: string
  added: number
  removed: number
  lines: DiffLine[]
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/

/// Lines that describe the diff itself rather than the file's contents: the group
/// header already names the path, and the counts come from the lines below.
const PLUMBING = [
  'index ',
  '--- ',
  '+++ ',
  'new file mode',
  'deleted file mode',
  'similarity index',
  'rename ',
  'old mode',
  'new mode',
  'Binary files',
  '\\ No newline',
]

/// Splits `git diff` output into per-file groups.
///
/// A file with no hunks is not an error: a pure rename or a binary file still
/// belongs in the list, it just has nothing to show when opened.
export function parseDiff(text: string): FileDiff[] {
  const files: FileDiff[] = []
  let current: FileDiff | null = null
  let oldNo = 0
  let newNo = 0

  for (const line of text.split('\n')) {
    if (line.startsWith('diff --git ')) {
      const match = /^diff --git a\/(.+) b\/(.+)$/.exec(line)
      current = { path: match ? match[2] : line.slice(11), added: 0, removed: 0, lines: [] }
      files.push(current)
      continue
    }
    if (!current) continue
    if (PLUMBING.some((prefix) => line.startsWith(prefix))) continue

    const hunk = HUNK.exec(line)
    if (hunk) {
      oldNo = Number(hunk[1])
      newNo = Number(hunk[2])
      current.lines.push({ kind: 'hunk', oldNo: null, newNo: null, text: line })
      continue
    }

    if (line.startsWith('+')) {
      current.added += 1
      current.lines.push({ kind: 'add', oldNo: null, newNo: newNo++, text: line.slice(1) })
      continue
    }
    if (line.startsWith('-')) {
      current.removed += 1
      current.lines.push({ kind: 'del', oldNo: oldNo++, newNo: null, text: line.slice(1) })
      continue
    }
    // Anything else is context, including the space-prefixed blank line git emits
    // between hunks. A bare empty string is the trailing newline of the output.
    if (line === '') continue
    current.lines.push({ kind: 'ctx', oldNo: oldNo++, newNo: newNo++, text: line.slice(1) })
  }

  return files
}
