import { memo, useMemo } from 'react'
import { Marked, Renderer } from 'marked'

import { openLink } from '../lib/ipc'

const escape = (text: string) => text.replace(/[&<>"']/g, (char) => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
})[char] ?? char)

const renderer = new Renderer()
renderer.html = ({ text }) => escape(text)
renderer.image = ({ text }) => escape(text)
renderer.link = function ({ href, tokens }) {
  const label = this.parser.parseInline(tokens)
  try {
    const url = new URL(href)
    if (url.protocol === 'http:' || url.protocol === 'https:') {
      return `<a href="${escape(url.href)}">${label}</a>`
    }
  } catch { /* A transcript has no base URL for relative links. */ }
  return label
}
const markdown = new Marked({ renderer, gfm: true, async: false })

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  const html = useMemo(() => markdown.parse(text, { async: false }), [text])
  return (
    <div className="markdown" dangerouslySetInnerHTML={{ __html: html }}
      onAuxClick={(event) => event.preventDefault()}
      onClick={(event) => {
        const link = (event.target as HTMLElement).closest('a')
        if (!link) return
        event.preventDefault()
        void openLink(link.href).catch(() => {})
      }} />
  )
})
