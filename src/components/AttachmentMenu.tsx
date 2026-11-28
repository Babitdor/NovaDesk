import { useId, useState } from 'react'
import { SettingsIcon } from './SettingsRows'
import { navigateComposerMenu } from '../lib/menu'

export type AttachmentKind = 'file' | 'folder' | 'project' | 'sketch'
const actions = [
  {id:'file', label:'Attach files', detail:'Choose one or more files from your computer.', icon:'file'},
  {id:'folder', label:'Attach a folder', detail:'Add a folder path without changing the workspace.', icon:'folder'},
  {id:'project', label:'Project files', detail:'Browse the Files pane and attach a saved file or folder.', icon:'folder'},
  {id:'sketch', label:'Sketch or mockup', detail:'Draw on a canvas and attach it as a PNG.', icon:'edit'},
] as const

export function AttachmentMenu({disabled, busy, onChoose}: {disabled:boolean; busy:boolean; onChoose:(kind:AttachmentKind)=>void}) {
  const id=useId()
  const [open,setOpen]=useState(false)
  return <>
    <button type="button" className="ghost composer-attach composer-picker-trigger" disabled={disabled || busy}
      aria-label="Add attachment" title={busy?'Choosing attachment…':'Attach files, a folder, project files or a sketch'}
      aria-busy={busy} aria-haspopup="menu" aria-expanded={open} aria-controls={id} popoverTarget={id}>
      <SettingsIcon name="attach" />
    </button>
    <div id={id} className="composer-menu attachment-menu" popover="auto" role="menu" aria-label="Add attachment"
      onToggle={event=>setOpen(event.newState==='open')} onKeyDown={navigateComposerMenu}>
      {actions.map((action,index)=><button key={action.id} type="button" role="menuitem" tabIndex={-1} data-kind={action.id}
        ref={button=>{if(index===0)button?.setAttribute('autofocus','')}} onClick={event=>{
          event.currentTarget.closest<HTMLElement>('[popover]')?.hidePopover()
          onChoose(action.id)
        }}><SettingsIcon name={action.icon}/><span><strong>{action.label}</strong><small>{action.detail}</small></span></button>)}
    </div>
  </>
}
