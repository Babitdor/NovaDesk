import { useEffect, useMemo, useRef } from 'react'
import { Marked, Renderer } from 'marked'

import { openLink, readFileDataUrl } from '../lib/ipc'
import { resolveRelative } from '../lib/preview'

const escape = (text: string) => text.replace(/[&<>"']/g, (char) => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
})[char] ?? char)

/// The transcript's renderer escapes every image and drops every relative link
/// because a transcript has no base URL. A preview of a file in the workspace
/// does: a relative href becomes a path the panel can open, and a relative
/// `src` becomes a data URL. That is a second renderer rather than a flag on the
/// first — the transcript's policy is a security property, not a preference.
function rendererFor(fromFile: string) {
  const renderer = new Renderer()
  // Raw HTML stays escaped here too: a previewed file must not be able to write
  // into the page that renders it.
  renderer.html = ({ text }) => escape(text)
  renderer.image = ({ href, text: alt }) => {
    const path = resolveRelative(fromFile, href)
    // A remote image would put a network request into a desktop preview; a
    // missing one leaves the alt text, which is the part a reader needs.
    return path === null
      ? escape(alt)
      : `<img data-asset="${escape(path)}" alt="${escape(alt)}">`
  }
  renderer.link = function ({ href, tokens }) {
    const label = this.parser.parseInline(tokens)
    // An anchor into the file on screen needs no interception: the webview
    // scrolls to it by itself.
    if (href.startsWith('#')) return `<a href="${escape(href)}">${label}</a>`
    if (/^https?:\/\//i.test(href)) return `<a href="${escape(href)}">${label}</a>`
    const path = resolveRelative(fromFile, href)
    return path === null ? label : `<a data-file="${escape(path)}">${label}</a>`
  }
  return renderer
}

/// Markdown with this file's own images and links working. The panel owns the
/// navigation: `onOpenFile` receives a workspace-relative path.
export function PreviewMarkdown({ text, from, onOpenFile }: {
  text: string
  /// Workspace-relative path of the file being rendered — every relative href in
  /// it resolves against this.
  from: string
  onOpenFile: (path: string) => void
}) {
  const html = useMemo(() => {
    const markdown = new Marked({ renderer: rendererFor(from), gfm: true, async: false })
    return markdown.parse(text, { async: false })
  }, [text, from])

  const body = useRef<HTMLDivElement>(null)

  // Images load after the markup exists, so a page with ten of them paints its
  // text first instead of waiting on ten IPC round trips.
  useEffect(() => {
    const node = body.current
    if (!node) return
    let alive = true
    for (const image of node.querySelectorAll<HTMLImageElement>('img[data-asset]')) {
      const path = image.dataset.asset
      if (!path) continue
      readFileDataUrl(path)
        .then((url) => {
          if (alive) image.src = url
        })
        // No src at all is what makes the browser draw the alt text, which is the
        // useful thing to show for an image that moved or is too large.
        .catch(() => {})
    }
    return () => {
      alive = false
    }
  }, [html])

  return (
    <div ref={body} className="markdown" dangerouslySetInnerHTML={{ __html: html }}
      onAuxClick={(event) => event.preventDefault()}
      onClick={(event) => {
        const link = (event.target as HTMLElement).closest('a')
        if (!link) return
        const file = link.dataset.file
        if (file) {
          event.preventDefault()
          onOpenFile(file)
          return
        }
        if (!/^https?:\/\//i.test(link.getAttribute('href') ?? '')) return
        event.preventDefault()
        void openLink(link.href).catch(() => {})
      }} />
  )
}
