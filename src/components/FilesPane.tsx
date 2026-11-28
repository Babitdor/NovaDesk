import { useState } from 'react'

import type { WorkspaceInfo } from '../lib/events'
import { FileTree } from './FileTree'
import { PreviewPanel } from './PreviewPanel'

/// The tree and the preview share one panel: opening a file replaces the tree and
/// closing it comes back, which is what keeps the pane one column wide.
///
/// `open` is how a tool opens a file: the request carries a nonce so asking for
/// the path that is already shown still counts as a new request.
export function FilesPane({ workspace, open }: {
  workspace: WorkspaceInfo | null
  open?: { path: string; nonce: number } | null
}) {
  const [selected, setSelected] = useState<string | null>(null)
  const [seen, setSeen] = useState(0)

  // Adjusting state during render, which is what React recommends for a prop
  // that should reset a view: doing it in an effect would paint the old file
  // once before the new one.
  if (open && open.nonce !== seen) {
    setSeen(open.nonce)
    setSelected(open.path)
  }

  if (!workspace) return <p className="hint">Open a folder to browse it.</p>

  if (selected) {
    return (
      <PreviewPanel path={selected} root={workspace.root} nonce={seen} onClose={() => setSelected(null)} />
    )
  }

  return <FileTree key={workspace.root} selected={selected} onSelectFile={setSelected} />
}
