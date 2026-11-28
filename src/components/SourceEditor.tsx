import { useEffect, useLayoutEffect, useImperativeHandle, useRef, useState, type Ref } from 'react'
import { basicSetup } from 'codemirror'
import { Compartment, EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { HighlightStyle, LanguageDescription, syntaxHighlighting } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { openSearchPanel, gotoLine } from '@codemirror/search'
import { tags } from '@lezer/highlight'
import { errorText, saveFile } from '../lib/ipc'
import { fileDraft, keepFileDraft } from '../lib/fileDrafts'

export type SourceHandle = { dirty: () => boolean; save: () => Promise<boolean>; discard: () => void }
type Props = { path:string; root:string; text:string; hidden:boolean; ref:Ref<SourceHandle>; onDirty:(dirty:boolean) => void; onSaved:() => void; onText:(text:string) => void }

const colors = HighlightStyle.define([
  { tag:tags.keyword, color:'var(--code-keyword)' },
  { tag:[tags.string, tags.special(tags.string)], color:'var(--code-string)' },
  { tag:[tags.number, tags.bool, tags.null], color:'var(--code-number)' },
  { tag:tags.comment, color:'var(--muted)', fontStyle:'italic' },
  { tag:[tags.function(tags.variableName), tags.definition(tags.variableName)], color:'var(--code-function)' },
  { tag:[tags.typeName, tags.className, tags.tagName], color:'var(--code-type)' },
  { tag:[tags.propertyName, tags.attributeName], color:'var(--code-property)' },
  { tag:tags.heading, color:'var(--accent)', fontWeight:'bold' },
  { tag:tags.link, color:'var(--code-property)', textDecoration:'underline' },
])

export function SourceEditor({ path, root, text, hidden, ref, onDirty, onSaved, onText }: Props) {
  const [initial] = useState(() => fileDraft(root, path) ?? { base:text, text })
  const base = useRef(initial.base)
  const document = useRef(initial.text)
  const lineEnding = useRef(initial.base.includes('\r\n') ? '\r\n' : '\n')
  const [ending, setEnding] = useState(initial.base.includes('\r\n') ? '\r\n' : '\n')
  const replacing = useRef(false)
  const disk = useRef(text)
  useLayoutEffect(() => { disk.current = text }, [text])
  const host = useRef<HTMLDivElement>(null)
  const editor = useRef<EditorView | null>(null)
  const wrap = useRef(new Compartment())
  const editable = useRef(new Compartment())
  const saving = useRef(false)
  const [dirty, setDirty] = useState(initial.base !== initial.text)
  const [busy, setBusy] = useState(false)
  const [wrapped, setWrapped] = useState(false)
  const [position, setPosition] = useState({ line:1, column:1 })
  const [message, setMessage] = useState(initial.base !== initial.text ? 'Recovered unsaved changes' : '')
  const [failure, setFailure] = useState('')
  const language = LanguageDescription.matchFilename(languages, path.replace(/\\/g, '/'))
  const updateDirty = useRef(onDirty)
  useLayoutEffect(() => { updateDirty.current = onDirty }, [onDirty])

  const remember = () => {
    const changed = document.current !== base.current
    setDirty(changed)
    onText(document.current)
    updateDirty.current(changed)
    try { keepFileDraft(root, path, base.current, document.current) }
    catch { setFailure('Draft recovery is unavailable. Save before leaving this workspace.'); }
  }
  const save = async () => {
    if (saving.current) return false
    if (document.current === base.current) return true
    saving.current = true
    setBusy(true)
    setFailure('')
    const content = document.current
    editor.current?.dispatch({ effects:editable.current.reconfigure(EditorState.readOnly.of(true)) })
    try {
      await saveFile(path, root, base.current, content)
      base.current = content
      disk.current = content
      remember()
      setMessage('Saved')
      onSaved()
      return true
    } catch (error) { setFailure(errorText(error)); return false }
    finally {
      saving.current = false
      setBusy(false)
      editor.current?.dispatch({ effects:editable.current.reconfigure(EditorState.readOnly.of(false)) })
    }
  }
  const actions = useRef({ save, remember })
  useLayoutEffect(() => { actions.current = { save, remember } })
  useImperativeHandle(ref, () => ({
    dirty:() => document.current !== base.current,
    save:() => actions.current.save(),
    discard:() => {
      base.current = disk.current
      document.current = disk.current
      lineEnding.current = disk.current.includes('\r\n') ? '\r\n' : '\n'
      setEnding(lineEnding.current)
      replacing.current = true
      editor.current?.dispatch({ changes:{ from:0, to:editor.current.state.doc.length, insert:disk.current } })
      replacing.current = false
      actions.current.remember()
      setFailure('')
      setMessage('Reloaded from disk')
    },
  }))

  useEffect(() => {
    if (!host.current) return
    let alive = true
    const syntax = new Compartment()
    const view = new EditorView({ parent:host.current, state:EditorState.create({
      doc:initial.text,
      extensions:[basicSetup, syntaxHighlighting(colors), syntax.of([]), wrap.current.of([]), editable.current.of([]),
        EditorView.contentAttributes.of({ 'aria-label':'File editor', spellcheck:'false', autocapitalize:'off' }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && !replacing.current) {
            document.current = update.state.doc.toString().replace(/\n/g, lineEnding.current)
            actions.current.remember()
          }
          if (update.selectionSet || update.docChanged) {
            const head = update.state.selection.main.head
            const line = update.state.doc.lineAt(head)
            setPosition({ line:line.number, column:head - line.from + 1 })
          }
        }),
        EditorView.domEventHandlers({ keydown:(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
            event.preventDefault(); void actions.current.save(); return true
          }
          return false
        } }),
      ],
    }) })
    editor.current = view
    actions.current.remember()
    if (language) void language.load().then((support) => {
      if (alive) view.dispatch({ effects:syntax.reconfigure(support) })
    }).catch(() => { if (alive) setMessage('Syntax unavailable; showing plain text') })
    return () => { alive = false; editor.current = null; view.destroy() }
    // A file owns one editor and undo history; refreshes update its document below.
  }, [path, root, initial, language])

  useEffect(() => {
    if (text === base.current) return
    if (document.current !== base.current) { setMessage('File changed on disk. Save will check for conflicts.'); return }
    base.current = text
    document.current = text
    lineEnding.current = text.includes('\r\n') ? '\r\n' : '\n'
    setEnding(lineEnding.current)
    const view = editor.current
    replacing.current = true
    view?.dispatch({ changes:{ from:0, to:view.state.doc.length, insert:text } })
    replacing.current = false
    actions.current.remember()
  }, [text])

  useEffect(() => { if (!hidden) editor.current?.requestMeasure() }, [hidden])

  return <section className="source-editor" hidden={hidden} aria-label="Source editor">
    <div className="source-toolbar" role="group" aria-label="Editor actions">
      <button type="button" className="ghost" onClick={() => editor.current && openSearchPanel(editor.current)} title="Find and replace (Ctrl+F / Ctrl+H)">Find</button>
      <button type="button" className="ghost" onClick={() => editor.current && gotoLine(editor.current)} title="Go to line (Ctrl+G)">Go to line</button>
      <button type="button" className="ghost" aria-pressed={wrapped} onClick={() => {
        editor.current?.dispatch({ effects:wrap.current.reconfigure(wrapped ? [] : EditorView.lineWrapping) }); setWrapped(!wrapped)
      }}>Wrap</button>
      <button type="button" className="source-save" disabled={!dirty || busy} onClick={() => void save()} title="Save file (Ctrl+S)">{busy ? 'Saving…' : 'Save'}</button>
    </div>
    <div className="code-editor" ref={host} />
    <footer className="source-status"><span>{dirty ? '● Unsaved' : 'Saved'}</span><span>Ln {position.line}, Col {position.column}</span><span>{language?.name ?? 'Plain text'}</span><span>UTF-8 · {ending === '\r\n' ? 'CRLF' : 'LF'}</span></footer>
    {(failure || message) && <p className={failure ? 'source-message is-error' : 'source-message'} role="status">{failure || message}</p>}
  </section>
}
