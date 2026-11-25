import { useEffect, useState } from 'react'

import { errorText, readFile } from '../lib/ipc'
import { Markdown } from './Markdown'

type Props = {
  path: string
  onClose: () => void
}

export function PreviewPanel({ path, onClose }: Props) {
  const [text, setText] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [raw, setRaw] = useState(false)
  const isMarkdown = /\.(?:md|markdown)$/i.test(path)

  useEffect(() => {
    let alive = true
    readFile(path)
      .then((content) => alive && setText(content))
      .catch((err) => alive && setError(errorText(err)))
    return () => {
      alive = false
    }
  }, [path])

  return (
    <section className="preview">
      <header className="preview-head">
        <span className="preview-path" title={path}>
          {path}
        </span>
        {isMarkdown && <button type="button" className="ghost" aria-pressed={raw} onClick={() => setRaw(!raw)}>
          {raw ? 'Preview' : 'Source'}
        </button>}
        <button type="button" className="ghost" onClick={onClose} aria-label="Close preview">
          ✕
        </button>
      </header>
      {error && <p className="panel-error">{error}</p>}
      {text !== null && (isMarkdown && !raw
        ? <div className="preview-body"><Markdown text={text} /></div>
        : <pre className="preview-body">{text}</pre>)}
    </section>
  )
}
