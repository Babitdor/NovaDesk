/// How the Files pane renders a file, from its extension alone — the one
/// decision the panel makes before it knows whether the read will work.
export type PreviewKind =
  | 'image'
  | 'markdown'
  | 'html'
  | 'pdf'
  | 'video'
  | 'audio'
  | 'text'

/// The same list the Rust side encodes to data URLs; a kind missing here is a
/// file the panel shows as text.
const IMAGES = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'ico', 'svg'])
/// Served by the loopback preview server instead: these are the kinds a player
/// scrubs in and a PDF viewer pages through, and a data URL cannot answer the
/// Range request either of them makes.
const VIDEO = new Set(['mp4', 'webm', 'mov', 'm4v', 'mkv'])
const AUDIO = new Set(['mp3', 'wav', 'ogg', 'oga', 'm4a', 'flac'])

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
  if (VIDEO.has(ext)) return 'video'
  if (AUDIO.has(ext)) return 'audio'
  if (ext === 'pdf') return 'pdf'
  if (ext === 'md' || ext === 'markdown') return 'markdown'
  if (ext === 'html' || ext === 'htm') return 'html'
  return 'text'
}

/// Whether the pane points a frame or a player at the loopback server: the bytes
/// are too big, or too ranged, to cross the IPC channel as a string.
export function servedByUrl(kind: PreviewKind): boolean {
  return kind === 'html' || kind === 'pdf' || kind === 'video' || kind === 'audio'
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

/// The glyph category the tree paints beside a name. Grouped one string per
/// category, so a new extension is a word in a list rather than another entry.
export type IconKind =
  | 'code'
  | 'page'
  | 'style'
  | 'doc'
  | 'data'
  | 'image'
  | 'media'
  | 'archive'
  | 'file'

const CATEGORIES: [IconKind, string][] = [
  ['code', 'ts tsx js jsx mjs cjs rs py go java rb c h cc cpp hpp cs php swift kt kts sh bash zsh ps1 bat cmd sql lua pl r scala dart ex exs erl hs ml clj el'],
  ['page', 'html htm vue svelte astro'],
  ['style', 'css scss sass less styl'],
  ['doc', 'md markdown txt rst log pdf adoc'],
  ['data', 'json jsonc json5 yaml yml toml ini cfg conf properties env csv tsv xml lock'],
  ['image', 'png jpg jpeg gif webp avif bmp ico svg'],
  ['media', 'mp4 webm mov m4v mkv mp3 wav ogg oga m4a flac'],
  ['archive', 'zip gz tar tgz bz2 xz 7z rar jar'],
]

const ICONS = new Map<string, IconKind>(
  CATEGORIES.flatMap(([kind, list]) => list.split(' ').map((ext): [string, IconKind] => [ext, kind])),
)

export function iconOf(path: string): IconKind {
  return ICONS.get(extension(path)) ?? 'file'
}

/// The path as breadcrumb segments, each with the prefix it stands for: the last
/// one is the file, the rest are folders the tree can be sent to.
export function crumbsOf(path: string): { name: string; path: string }[] {
  const parts = path.split('/').filter(Boolean)
  return parts.map((name, index) => ({ name, path: parts.slice(0, index + 1).join('/') }))
}
