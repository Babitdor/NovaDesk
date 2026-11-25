import type { WorkspaceInfo } from '../lib/events'
import { panelClearance, panelLimit, panelMax, panelMin } from '../lib/layout'
import { PANES, type Pane } from '../lib/panes'
import { BrowserPane } from './BrowserPane'
import { ChangesPane } from './ChangesPane'
import { FilesPane } from './FilesPane'
import { Splitter } from './Splitter'

type Props = {
  workspace: WorkspaceInfo | null
  tab: Pane
  open: boolean
  obscured: boolean
  width: number
  sidebarWidth: number
  onSelect: (pane: Pane) => void
  onWidth: (width: number) => void
}

/// Every pane stays mounted and is hidden with CSS: the browser keeps its page
/// across tab switches. The panel's own width is the only thing `open` changes,
/// so collapsing it never costs a remount.
export function RightPanel({ workspace, tab, open, obscured, width, sidebarWidth, onSelect, onWidth }: Props) {
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
      <div className="tabs" role="tablist" aria-label="Panel">
        {PANES.map((name) => (
          <button
            key={name}
            type="button"
            role="tab"
            aria-selected={name === tab}
            className={name === tab ? 'tab is-active' : 'tab'}
            onClick={() => onSelect(name)}
          >
            {name}
          </button>
        ))}
      </div>

      <div className={pane('Changes')} role="tabpanel" aria-label="Changes">
        <ChangesPane workspace={workspace} />
      </div>
      <div className={pane('Files')} role="tabpanel" aria-label="Files">
        <FilesPane key={workspace?.root} workspace={workspace} />
      </div>
      <div className={pane('Browser')} role="tabpanel" aria-label="Browser">
        <BrowserPane active={open && !obscured && tab === 'Browser'} />
      </div>
    </section>
  )
}
