import { useEffect, useLayoutEffect, useRef, useState, lazy, Suspense } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { SourceHandle } from './SourceEditor'
const SourceEditor = lazy(() => import('./SourceEditor').then((module) => ({ default:module.SourceEditor })))

import { errorText, fileStamp, previewOpen, readFile, readFileDataUrl } from '../lib/ipc'
import { kindOf } from '../lib/preview'
import { PreviewMarkdown } from './PreviewMarkdown'

type Props = {
  path: string
  root: string
  nonce: number
  onClose: () => void
}

/// One load's result. `for` is the request it answers, so content belonging to a
/// file the reader has already left is simply not painted.
type Loaded = {
  for: string
  path: string
  text: string | null
  asset: string | null
  frame: string | null
  error: string | null
}

const EMPTY: Loaded = { for: '', path:'', text: null, asset: null, frame: null, error: null }

/// The pane's preview, the way VS Code does it: an image paints, Markdown renders
/// with its own images and links, HTML runs in a loopback-origined frame with its
/// own CSS and scripts, and everything else is text. Preview ⇄ Source exists only
/// where a source view means something, which is Markdown and HTML.
export function PreviewPanel({ path, root, nonce, onClose }: Props) {
  // The panel navigates itself: a link inside the preview moves this path, not
  // the file tree, because the pane is one column wide.
  const [current, setCurrent] = useState(path)
  const [source, setSource] = useState(false)
  const [editingText, setEditingText] = useState<string | null>(null)
  const editor = useRef<SourceHandle>(null)
  const [dirty, setDirty] = useState(false)
  const [pending, setPending] = useState<{ run:() => void; cancel?:() => void } | null>(null)
  const [leaving, setLeaving] = useState(false)
  const [leaveError, setLeaveError] = useState('')
  const dialog = useRef<HTMLDialogElement>(null)
  const requestLeave = (run:() => void) => {
    if (editor.current?.dirty()) { setLeaveError(''); setPending({ run }) }
    else run()
  }
  const navigate = (target:string) => requestLeave(() => { setCurrent(target); setSource(false); setDirty(false); setEditingText(null) })
  const navigateRef = useRef(navigate)
  useLayoutEffect(() => { navigateRef.current = navigate })
  const lastPath = useRef({ path, nonce })
  useEffect(() => {
    if (lastPath.current.path === path && lastPath.current.nonce === nonce) return
    lastPath.current = { path, nonce }
    navigateRef.current(path)
  }, [path, nonce])
  const leaveRef = useRef(requestLeave)
  useLayoutEffect(() => { leaveRef.current = requestLeave })
  useEffect(() => {
    let alive = true
    let allowed = false
    const unlisten = getCurrentWindow().onCloseRequested((event) => {
      if (allowed || !editor.current?.dirty()) return
      event.preventDefault()
      leaveRef.current(() => { allowed = true; void getCurrentWindow().close() })
    })
    void unlisten.then((stop) => { if (!alive) stop() }).catch(() => {})
    return () => { alive = false; void unlisten.then((stop) => stop()).catch(() => {}) }
  }, [])
  useEffect(() => {
    const guard = (event:Event) => {
      if (!editor.current?.dirty()) return
      event.preventDefault()
      const complete = (event as CustomEvent<(allowed:boolean) => void>).detail
      setLeaveError('')
      setPending({ run:() => complete(true), cancel:() => complete(false) })
    }
    window.addEventListener('nova:before-workspace-change', guard)
    return () => window.removeEventListener('nova:before-workspace-change', guard)
  }, [])
  useEffect(() => {
    if (pending && dialog.current && !dialog.current.open) dialog.current.showModal()
  }, [pending])
  // Bumped when the file changes on disk: it re-runs the load effect and remounts
  // the frame, which is the whole refresh mechanism.
  const [version, setVersion] = useState(0)
  const [loaded, setLoaded] = useState<Loaded>(EMPTY)
  const kind = kindOf(current)
  const canToggle = kind === 'markdown' || kind === 'html'

  // Keep the editor mounted during refreshes so its selection and undo survive.
  // Results for a different path still never paint in this file's pane.
  const request = `${current}\n${version}\n${source}`
  const shown = loaded.path === current ? loaded : EMPTY
  const { text, asset, frame, error } = shown

  useEffect(() => {
    let alive = true
    // Two independent commands feed one panel, so a delivery merges into the
    // current request instead of replacing it — the late one must not wipe the
    // other's field.
    const deliver = (partial: Partial<Omit<Loaded, 'for' | 'path'>>) => {
      if (!alive) return
      setLoaded((last) => ({
        ...(last.path === current ? last : EMPTY),
        for:request, path:current,
        ...partial,
      }))
    }

    if (kind === 'image') {
      readFileDataUrl(current).then(
        (url) => deliver({ asset: url }),
        (err) => deliver({ error: errorText(err) }),
      )
    } else {
      readFile(current).then(
        (content) => deliver({ text: content }),
        (err) => deliver({ error: errorText(err) }),
      )
      // Only the framed view needs a server URL; the source view is plain text.
      if (kind === 'html' && !source) {
        previewOpen(current).then(
          (url) => deliver({ frame: url }),
          (err) => deliver({ error: errorText(err) }),
        )
      }
    }
    return () => {
      alive = false
    }
  }, [request, current, kind, source])

  // shortcut: one metadata IPC per second while a preview is open. A real file
  // watcher is the upgrade once that cost matters; this only asks for a length
  // and an mtime, never the bytes.
  useEffect(() => {
    let seen: string | null = null
    let alive = true
    // The baseline is read now, not on the first tick: a file that changes
    // within the first second would otherwise be adopted as the baseline and
    // never painted.
    const check = () => {
      fileStamp(current)
        .then((stamp) => {
          if (!alive) return
          const key = `${stamp.len}:${stamp.modified}`
          if (seen === null) {
            seen = key
            return
          }
          if (key === seen) return
          seen = key
          setVersion((count) => count + 1)
        })
        .catch(() => {})
    }
    check()
    const timer = setInterval(check, 1000)
    return () => {
      alive = false
      clearInterval(timer)
    }
  }, [current])

  return (
    <section className="preview">
      <header className="preview-head">
        <span className="preview-path" title={current}>
          {current}
        </span>
        <button type="button" className="ghost" title="Reload file from disk" onClick={() => requestLeave(() => { editor.current?.discard(); setVersion((value) => value + 1) })}>Reload</button>
        {canToggle && <button type="button" className="ghost" aria-pressed={source}
          onClick={() => setSource(!source)}>
          {source ? 'Preview' : 'Source'}
        </button>}
        <button type="button" className="ghost" onClick={() => requestLeave(onClose)} aria-label="Close preview">
          ✕
        </button>
      </header>
      {dirty && <p className="preview-dirty" role="status">Unsaved changes · draft kept for recovery{kind === 'html' && !source ? ' · HTML preview shows the saved file' : ''}</p>}
      {error && <p className="panel-error" role="status">{error}</p>}
      {!error && !asset && text === null && <p className="hint" role="status">Loading file…</p>}
      {kind === 'image' && asset && (
        <div className="preview-body">
          <img className="preview-image" src={asset} alt={current} />
        </div>
      )}
      {/* The frame is another origin, so it reaches no IPC and no app state;
          sandbox is what keeps it there even when its own markup objects. */}
      {kind === 'html' && !source && frame && (
        <iframe key={`${frame}:${version}`} className="preview-frame" src={frame} title={current}
          sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals" />
      )}
      {text !== null && !text.includes('\0') && <Suspense fallback={<p className="hint">Loading editor…</p>}><SourceEditor key={current} ref={editor} path={current} root={root} text={text}
        hidden={canToggle && !source} onDirty={setDirty} onText={setEditingText} onSaved={() => setVersion((value) => value + 1)} /></Suspense>}
      {text?.includes('\0') && <p className="panel-error" role="status">This is a binary file. Text editing is unavailable.</p>}
      {kind === 'markdown' && !source && text !== null && (
        <div className="preview-body">
          <PreviewMarkdown text={editingText ?? text} from={current} onOpenFile={navigate} />
        </div>
      )}
      {pending && <dialog className="unsaved-dialog" ref={dialog} aria-labelledby="unsaved-title" onCancel={(event) => { event.preventDefault(); if (!leaving) { pending.cancel?.(); setPending(null) } }}>
        <h2 id="unsaved-title">Save changes?</h2>
        <p>{current} has unsaved changes.</p>
        {leaveError && <p role="status">{leaveError}</p>}
        <div>
          <button type="button" disabled={leaving} onClick={() => { pending.cancel?.(); setPending(null) }}>Keep editing</button>
          <button type="button" disabled={leaving} onClick={() => { editor.current?.discard(); const run = pending.run; setPending(null); run() }}>Discard changes</button>
          <button type="button" disabled={leaving} onClick={async () => {
            setLeaving(true)
            if (await editor.current?.save()) { const run = pending.run; setPending(null); run() }
            else setLeaveError('Could not save. Keep editing to review the error; your draft is still available.')
            setLeaving(false)
          }}>{leaving ? 'Saving…' : 'Save and continue'}</button>
        </div>
      </dialog>}
    </section>
  )
}
