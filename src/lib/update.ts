/// The About pane's update line, without any arithmetic in the component.
///
/// Two properties of the data make this worth a module of its own. The
/// percentage can be absent — a chunked response sends no length — and the
/// plugin reports each chunk rather than a running total, so every number here
/// derives from what Rust summed. Both are checked by `node --test` instead of
/// by watching a progress bar and hoping.

import type { UpdateProgress } from './ipc'

const MB = 1024 * 1024

/// Whole percent for the progress line, or `null` when the server sent no
/// length. The pane then prints bytes and no bar rather than inventing a
/// denominator.
export function progressPercent(progress: UpdateProgress | null): number | null {
  if (!progress || progress.stage !== 'downloading') return null
  // `0` is falsy and also means "the server did not say": it would divide by
  // zero, so it is treated exactly like a missing total.
  if (!progress.total) return null
  const percent = Math.floor((progress.downloaded / progress.total) * 100)
  // A proxy that appends bytes can push the count past the advertised length;
  // "103%" would read as a bug in the pane rather than in the network.
  return Math.max(0, Math.min(100, percent))
}

/// One decimal below 100 MB, whole numbers above: the progress line is read at a
/// glance, and the digit that moves is the interesting one.
function megabytes(bytes: number): string {
  const value = bytes / MB
  return value >= 100 ? `${Math.round(value)} MB` : `${value.toFixed(1)} MB`
}

/// The single line the pane shows while an update runs. `installing` is
/// terminal: on Windows the plugin replaces this process, so this sentence is
/// the last thing a reader sees before the window closes.
export function progressLabel(progress: UpdateProgress | null): string {
  if (!progress) return ''
  switch (progress.stage) {
    case 'checking':
      return 'Checking GitHub for a newer release…'
    case 'downloading': {
      const percent = progressPercent(progress)
      const size = progress.total
        ? `${megabytes(progress.downloaded)} of ${megabytes(progress.total)}`
        : megabytes(progress.downloaded)
      return percent === null ? `Downloading ${size}` : `Downloading ${size} · ${percent}%`
    }
    case 'installing':
      return 'Installing — Nova closes and reopens on the new version.'
  }
}
