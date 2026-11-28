import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { errorText, saveSketch } from '../lib/ipc'

type Point = {x:number; y:number}
type Tool = 'pencil' | 'rectangle' | 'line' | 'text'
type Shape = {tool:Tool; points:Point[]; color:string; width:number; text:string}
const WIDTH=1000, HEIGHT=600

export function SketchDialog({root,onAttach,onClose}: {root:string;onAttach:(path:string)=>void;onClose:()=>void}) {
  const dialog=useRef<HTMLDialogElement>(null)
  const canvas=useRef<HTMLCanvasElement>(null)
  const [shapes,setShapes]=useState<Shape[]>([])
  const latest=useRef<Shape[]>([])
  const [history,setHistory]=useState<Shape[][]>([])
  const active=useRef<number|null>(null)
  const pointer=useRef<number|null>(null)
  const [tool,setTool]=useState<Tool>('pencil')
  const [color,setColor]=useState('#20242b')
  const [width,setWidth]=useState(4)
  const [text,setText]=useState('Label')
  const [cursor,setCursor]=useState<Point>({x:30,y:30})
  const [focused,setFocused]=useState(false)
  const [saving,setSaving]=useState(false)
  const [error,setError]=useState('')
  const [discard,setDiscard]=useState(false)
  const [windowClosing,setWindowClosing]=useState(false)
  const allowClose=useRef(false)
  const commit=(next:Shape[])=> {latest.current=next;setShapes(next)}
  const checkpoint=()=> {const snapshot=latest.current;setHistory(previous=>[...previous,snapshot])}
  const undo=()=> {
    const previous=history.at(-1)
    if(!previous)return
    active.current=null
    commit(previous)
    setHistory(history.slice(0,-1))
  }
  const begin=(point:Point)=> {
    checkpoint()
    active.current=tool==='text'?null:latest.current.length
    commit([...latest.current,{tool,points:[point],color,width,text}])
  }
  const move=(point:Point)=> {
    const index=active.current
    if(index===null)return
    const shape=latest.current[index]
    if(!shape)return
    const updated={...shape,points:shape.tool==='pencil'?[...shape.points,point]:[shape.points[0],point]}
    commit(latest.current.map((entry,at)=>at===index?updated:entry))
  }
  const paint=useCallback((showCursor:boolean)=> {
    const node=canvas.current
    const context=node?.getContext('2d')
    if(!node||!context)return
    context.fillStyle='#ffffff'
    context.fillRect(0,0,WIDTH,HEIGHT)
    context.lineCap='round'
    context.lineJoin='round'
    for(const shape of latest.current) {
      context.strokeStyle=shape.color
      context.fillStyle=shape.color
      context.lineWidth=shape.width
      const first=shape.points[0], last=shape.points.at(-1)!
      context.beginPath()
      if(shape.tool==='text') {
        context.font='20px system-ui'
        context.fillText(shape.text,first.x,first.y)
      } else if(shape.tool==='rectangle') {
        context.strokeRect(Math.min(first.x,last.x),Math.min(first.y,last.y),Math.abs(last.x-first.x),Math.abs(last.y-first.y))
      } else if(shape.points.length===1) {
        context.arc(first.x,first.y,shape.width/2,0,Math.PI*2);context.fill()
      } else {
        context.moveTo(first.x,first.y)
        for(const point of shape.points.slice(1))context.lineTo(point.x,point.y)
        context.stroke()
      }
    }
    if(showCursor&&focused) {
      context.strokeStyle='#607080';context.lineWidth=1
      context.beginPath();context.moveTo(cursor.x-7,cursor.y);context.lineTo(cursor.x+7,cursor.y)
      context.moveTo(cursor.x,cursor.y-7);context.lineTo(cursor.x,cursor.y+7);context.stroke()
    }
  },[cursor,focused])
  useLayoutEffect(()=>paint(true),[shapes,paint])
  useLayoutEffect(()=> {dialog.current?.showModal()},[])
  useEffect(()=> {
    let stale=false,off:(()=>void)|undefined
    void getCurrentWindow().onCloseRequested(event=> {
      if(latest.current.length&&!allowClose.current) {
        event.preventDefault();setWindowClosing(true);setDiscard(true)
      }
    }).then(dispose=>{if(stale)dispose();else off=dispose}).catch(()=>{})
    return()=>{stale=true;off?.()}
  },[])
  const close=()=> {dialog.current?.close();onClose()}
  const requestClose=()=> {if(saving)return;if(latest.current.length)setDiscard(true);else close()}
  const pointAt=(clientX:number,clientY:number):Point=> {
    const rect=canvas.current!.getBoundingClientRect()
    return {x:Math.max(0,Math.min(WIDTH,(clientX-rect.left)*WIDTH/rect.width)),y:Math.max(0,Math.min(HEIGHT,(clientY-rect.top)*HEIGHT/rect.height))}
  }
  const attach=async()=> {
    if(saving||!latest.current.length)return
    setSaving(true);setError('');active.current=null
    try {
      paint(false)
      const png=canvas.current!.toDataURL('image/png')
      paint(true)
      const path=await saveSketch(root,png)
      dialog.current?.close()
      onAttach(path)
    } catch(err) {setError(errorText(err));setSaving(false)}
  }
  return <dialog ref={dialog} className="sketch-dialog" aria-labelledby="sketch-title"
    onCancel={event=>{event.preventDefault();requestClose()}}>
    <header><h2 id="sketch-title">Sketch or mockup</h2><button type="button" className="ghost" onClick={requestClose} disabled={saving}>Cancel</button></header>
    <fieldset className="sketch-tools" disabled={saving}>
      <legend className="sr-only">Drawing tools</legend>
      <div role="group" aria-label="Drawing tool">{(['pencil','rectangle','line','text'] as const).map(item=><button type="button" key={item}
        aria-pressed={tool===item} onClick={()=>{active.current=null;setTool(item)}}>{item[0].toUpperCase()+item.slice(1)}</button>)}</div>
      <label>Color<input type="color" value={color} onChange={event=>setColor(event.target.value)}/></label>
      <label>Stroke<select value={width} onChange={event=>setWidth(Number(event.target.value))}>{[2,4,8,12].map(size=><option key={size} value={size}>{size} px</option>)}</select></label>
      {tool==='text'&&<label>Label<input aria-label="Canvas label" value={text} maxLength={120} onChange={event=>setText(event.target.value)}/></label>}
      <button type="button" disabled={!history.length} onClick={undo}>Undo</button>
      <button type="button" disabled={!shapes.length} onClick={()=>{checkpoint();active.current=null;commit([])}}>Clear</button>
    </fieldset>
    <div className="sketch-stage"><canvas ref={canvas} width={WIDTH} height={HEIGHT} tabIndex={0} aria-disabled={saving}
      aria-label="Drawing canvas" aria-describedby="sketch-help"
      onFocus={()=>setFocused(true)} onBlur={()=>setFocused(false)}
      onPointerDown={event=> {
        if(saving||!event.isPrimary||event.button!==0)return
        canvas.current?.focus({preventScroll:true});canvas.current?.setPointerCapture(event.pointerId)
        pointer.current=event.pointerId
        const point=pointAt(event.clientX,event.clientY);setCursor(point);begin(point)
      }}
      onPointerMove={event=> {
        if(saving||pointer.current!==event.pointerId)return
        const point=pointAt(event.clientX,event.clientY);setCursor(point);move(point)
      }}
      onPointerUp={()=>{pointer.current=null;active.current=null}}
      onPointerCancel={()=>{pointer.current=null;active.current=null}}
      onKeyDown={event=> {
        if(saving)return
        if((event.ctrlKey||event.metaKey)&&event.key.toLowerCase()==='z'){event.preventDefault();undo();return}
        const steps:Record<string,Point>={ArrowLeft:{x:-1,y:0},ArrowRight:{x:1,y:0},ArrowUp:{x:0,y:-1},ArrowDown:{x:0,y:1}}
        if(steps[event.key]) {
          event.preventDefault()
          const step=event.shiftKey?20:5,delta=steps[event.key]
          const point={x:Math.max(0,Math.min(WIDTH,cursor.x+delta.x*step)),y:Math.max(0,Math.min(HEIGHT,cursor.y+delta.y*step))}
          setCursor(point);move(point)
        } else if(event.key===' '||event.key==='Enter') {
          event.preventDefault();if(active.current===null)begin(cursor);else active.current=null
        }
      }}/></div>
    <p id="sketch-help">Draw with your mouse or pen. Keyboard: arrows move, Shift moves farther, Space starts/ends a shape, Ctrl+Z undoes.</p>
    {error&&<p role="alert" className="sketch-error">{error}</p>}
    <footer>{discard?<>
      <span>{windowClosing?'Discard this drawing and close the window?':'Discard this drawing?'}</span>
      <button type="button" onClick={()=>{setDiscard(false);setWindowClosing(false)}}>Keep drawing</button>
      <button type="button" onClick={()=>{
        if(windowClosing){allowClose.current=true;void getCurrentWindow().close().catch(err=>setError(errorText(err)))}
        else close()
      }}>Discard</button>
    </>:<>
      <span>Saved as a PNG in .nova/attachments when attached.</span>
      <button type="button" className="send" disabled={saving||!shapes.length} onClick={()=>void attach()}>{saving?'Attaching…':'Attach drawing'}</button>
    </>}</footer>
  </dialog>
}
