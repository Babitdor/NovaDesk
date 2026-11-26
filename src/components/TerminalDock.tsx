import { useId, useRef, useState } from 'react'
import { terminalShells, type TerminalShell } from '../lib/ipc'
import { dockMax, dockMin } from '../lib/layout'
import { ConsolePanel } from './ConsolePanel'
import { InteractiveTerminal } from './InteractiveTerminal'
import { Splitter } from './Splitter'

export type Dock = 'open' | 'closed' | 'max'

type Props = {
  dock: Dock
  height: number | null
  onDock: (next: Dock) => void
  onHeight: (height: number) => void
  workspaceReady: boolean
  obscured: boolean
  /// A tool asked for a shell. The nonce makes the same shell a new tab.
  spawn?: { shell: TerminalShell; nonce: number } | null
}

export function TerminalDock({ dock, height, onDock, onHeight, workspaceReady, obscured, spawn }: Props) {
  const [terminals, setTerminals] = useState<{ id: number; shell: TerminalShell }[]>([])
  const [selected, select] = useState<number | null>(null)
  const [next, setNext] = useState(1)
  const [seenSpawn, setSeenSpawn] = useState(0)
  const [shellMenu, setShellMenu] = useState(false)
  const shellMenuId = useId()
  const menuButton = useRef<HTMLButtonElement>(null)
  const closed = dock === 'closed'
  const sized = !closed && dock !== 'max' && height !== null

  if (!closed && workspaceReady && next === 1) {
    setTerminals([{ id: 1, shell: 'powershell' }])
    select(1)
    setNext(2)
  }

  // A tool's open_terminal lands here: the dock owns its tabs, so it adds one
  // rather than being told about a terminal it never created. Done during
  // render, like the first-terminal seed above, so the tab exists in the same
  // paint as the request.
  if (spawn && spawn.nonce !== seenSpawn) {
    setSeenSpawn(spawn.nonce)
    const id = next
    setTerminals((current) => [...current, { id, shell: spawn.shell }])
    select(id)
    setNext(id + 1)
  }

  return (
    <section
      className={closed ? 'dock is-closed' : dock === 'max' ? 'dock is-max' : 'dock'}
      style={sized ? { height, maxHeight: 'none' } : undefined}
    >
      {!closed && dock !== 'max' && (
        <Splitter
          axis="y"
          dir={-1}
          value={height}
          min={dockMin}
          max={dockMax}
          label="Resize the terminal"
          onChange={onHeight}
        />
      )}
      <header className="dock-head">
        <span className="dock-symbol" aria-hidden="true">›_</span>
        <div className="dock-tabs" role="group" aria-label="Terminal sessions">
          <button type="button" className="ghost dock-tab" aria-pressed={selected === null} onClick={() => select(null)}>Console</button>
          {terminals.map(({ id, shell }, index) => <span className="dock-terminal-tab" key={id}>
            <button type="button" className="ghost dock-tab" aria-pressed={selected === id} onClick={() => select(id)}>{terminalShells[shell]} {id}</button>
            <button type="button" className="ghost" aria-label={`Close shell ${id}`} onClick={() => {
              setTerminals((current) => current.filter((tab) => tab.id !== id))
              if (selected === id) select(terminals[index + 1]?.id ?? terminals[index - 1]?.id ?? null)
            }}>×</button>
          </span>)}
        </div>
        <span className="dock-spacer" />
        <div className="dock-shell-picker" onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) setShellMenu(false)
        }} onKeyDown={(event) => {
          if (event.key !== 'Escape' || !shellMenu) return
          event.stopPropagation()
          setShellMenu(false)
          menuButton.current?.focus()
        }}>
          <button type="button" className="ghost" ref={menuButton} aria-label="New terminal"
            aria-expanded={shellMenu} aria-controls={shellMenu ? shellMenuId : undefined}
            disabled={!workspaceReady || terminals.length >= 5}
            onClick={() => setShellMenu((open) => !open)}>+</button>
          {shellMenu && !closed && <div className="dock-shell-options" id={shellMenuId} role="group" aria-label="Terminal shells">
            {(Object.keys(terminalShells) as TerminalShell[]).map((shell) => (
              <button type="button" className="ghost" key={shell} onClick={() => {
                setTerminals((current) => [...current, { id: next, shell }])
                select(next)
                setNext(next + 1)
                setShellMenu(false)
                menuButton.current?.focus()
              }}>{terminalShells[shell]}</button>
            ))}
          </div>}
        </div>
        <button
          type="button"
          className="ghost"
          aria-label={dock === 'max' ? 'Restore terminal size' : 'Maximize the terminal'}
          onClick={() => onDock(dock === 'max' ? 'open' : 'max')}
        >
          {dock === 'max' ? 'Restore' : 'Max'}
        </button>
        <button
          type="button"
          className="ghost"
          aria-label="Close the terminal"
          onClick={() => onDock('closed')}
        >
          ×
        </button>
      </header>

      {/* Mounted while closed: the console's scrollback is the reason it exists. */}
      <div className="dock-body">
        <div className="dock-console" hidden={selected !== null}><ConsolePanel /></div>
        {terminals.map(({ id, shell }) => <InteractiveTerminal key={id} shell={shell} active={selected === id && !closed && !obscured} />)}
      </div>
    </section>
  )
}
