/// How the Files pane renders a file, from its extension alone — the one
/// decision the panel makes before it knows whether the read will work.
export type PreviewKind = 'image' | 'markdown' | 'html' | 'text'

/// The same list the Rust side encodes to data URLs; a kind missing here is a
/// file the panel shows as text.
const IMAGES = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'ico', 'svg'])

/// The extension of the last path segment, lowercased; '' for a dotfile or a
/// name with no dot. A leading dot is part of the name ('.gitignore'), not an
/// extension, which is why the search stops at 0.
function extension(path: string): string {
  const name = path.slice(path.lastIndexOf('/') + 1)
  const dot = name.lastIndexOf('.')
  return dot <= 0 ? '' : name.slice(dot + 1).toLowerCase()
}

export function kindOf(path: string): PreviewKind {
  const ext = extension(path)
  if (IMAGES.has(ext)) return 'image'
  if (ext === 'md' || ext === 'markdown') return 'markdown'
  if (ext === 'html' || ext === 'htm') return 'html'
  return 'text'
}

/// Turns a Markdown href or an `src` into a workspace-relative path, or null
/// when it is not workspace content.
///
/// Null covers an absolute URL, a data: image, a bare `#anchor`, and anything
/// that climbs out of the root — the panel's links and image sources both go
/// through here, so a link in a previewed file can never open something the file
/// tree would not have shown.
export function resolveRelative(fromFile: string, href: string): string | null {
  const raw = href.trim()
  // `//host/x` is protocol-relative and `scheme:` is someone else's; a raw '#'
  // is an anchor inside the file already on screen.
  if (!raw || raw.startsWith('//') || /^[a-z][a-z0-9+.-]*:/i.test(raw)) return null
  // A query or fragment is a cache-buster or a scroll target, not part of a name.
  const cut = raw.split(/[?#]/)[0]
  if (!cut) return null

  let target: string
  try {
    target = decodeURIComponent(cut)
  } catch {
    // A stray % is a filename, not a broken encoding.
    target = cut
  }
  target = target.replace(/\\/g, '/')

  // A leading slash means the workspace root, which is the only root a preview
  // of this workspace has.
  const stack: string[] = target.startsWith('/') ? [] : fromFile.split('/').slice(0, -1)
  for (const segment of target.split('/')) {
    if (segment === '' || segment === '.') continue
    if (segment === '..') {
      if (stack.length === 0) return null
      stack.pop()
      continue
    }
    stack.push(segment)
  }
  return stack.length > 0 ? stack.join('/') : null
}
