import type { WorkspaceInfo } from '../lib/events'
import { panelClearance, panelLimit, panelMax, panelMin } from '../lib/layout'
import type { Pane } from '../lib/panes'
import type { Chat, Column } from '../lib/sessions'
import type { Preferences } from '../lib/settings'
import { BoardPane } from './BoardPane'
import { BrowserPane, type BrowserTarget } from './BrowserPane'
import { FilesPane } from './FilesPane'
import { GitHubPane } from './GitHubPane'
import { Splitter } from './Splitter'

type Props = {
  workspace: WorkspaceInfo | null
  tab: Pane
  open: boolean
  obscured: boolean
  width: number
  sidebarWidth: number
  chats: Chat[]
  activeId: string
  preferences: Preferences
  onOpenChat: (id: string) => void
  onMove: (id: string, column: Column) => void
  onWidth: (width: number) => void
  onUpdatePreferences: (patch: Partial<Preferences>) => void
  /// A clone hands back a folder; the shell is the only owner of the workspace.
  onOpenFolder: (path: string) => void
  /// Requests a tool made that this panel carries out: a file to show in the
  /// tree's preview, a page the browser child should navigate to.
  fileToOpen: { path: string; nonce: number } | null
  browserTarget: BrowserTarget | null
}

/// Every pane stays mounted and is hidden with CSS: the browser keeps its page
/// and the diff keeps its scroll across view switches. The panel's own width is
/// the only thing `open` changes, so collapsing it never costs a remount.
///
/// The switcher for these panes is not here: it sits in the shell's top row, so
/// it is still there when the panel is closed — which is what lets selecting a
/// pane reopen it.
export function RightPanel({
  workspace, tab, open, obscured, width, sidebarWidth, chats, activeId, preferences,
  onOpenChat, onMove, onWidth, onUpdatePreferences, onOpenFolder, fileToOpen, browserTarget,
}: Props) {
  const pane = (name: Pane) => (name === tab ? 'pane' : 'pane is-hidden')

  return (
    <section className={open ? 'right' : 'right is-closed'} inert={!open}
      style={open ? { width, maxWidth: `clamp(${panelMin}px, calc(100vw - ${sidebarWidth + panelClearance}px), ${panelLimit}px)` } : undefined}>
      {open && (
        <Splitter
          axis="x"
          dir={-1}
          value={Math.min(width, panelMax(sidebarWidth))}
          min={panelMin}
          max={() => panelMax(sidebarWidth)}
          label="Resize the panel"
          onChange={onWidth}
        />
      )}

      <div className={pane('Files')} role="tabpanel" aria-label="Files">
        <FilesPane key={workspace?.root} workspace={workspace} open={fileToOpen} />
      </div>
      <div className={pane('GitHub')} role="tabpanel" aria-label="GitHub">
        <GitHubPane workspace={workspace} preferences={preferences}
          update={onUpdatePreferences} onOpenFolder={onOpenFolder} />
      </div>
      <div className={pane('Board')} role="tabpanel" aria-label="Board">
        <BoardPane chats={chats} activeId={activeId} onOpen={onOpenChat} onMove={onMove} />
      </div>
      <div className={pane('Preview')} role="tabpanel" aria-label="Preview">
        <BrowserPane active={open && !obscured && tab === 'Preview'} target={browserTarget} />
      </div>
    </section>
  )
}
