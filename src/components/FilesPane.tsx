import { useState } from 'react'

import type { WorkspaceInfo } from '../lib/events'
import { FileTree } from './FileTree'
import { PreviewPanel } from './PreviewPanel'

/// The tree and the preview share one panel: opening a file replaces the tree and
/// closing it comes back, which is what keeps the pane one column wide.
export function FilesPane({ workspace }: { workspace: WorkspaceInfo | null }) {
  const [selected, setSelected] = useState<string | null>(null)

  if (!workspace) return <p className="hint">Open a folder to browse it.</p>

  if (selected) {
    return <PreviewPanel key={selected} path={selected} onClose={() => setSelected(null)} />
  }

  return <FileTree key={workspace.root} selected={selected} onSelectFile={setSelected} />
}
