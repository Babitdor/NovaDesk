import { useRef } from 'react'

type Props = {
  axis: 'x' | 'y'
  /// The direction the pane grows when the handle moves: 1 when the handle is on
  /// a pane's far edge (the rail), -1 when it is on its near edge (the panel and
  /// the dock, which grow as the pointer moves left or up).
  dir?: 1 | -1
  /// The size the shell holds. `null` means the pane is still auto-sized, which
  /// the first drag measures instead.
  value: number | null
  min: number
  max: () => number
  label: string
  onChange: (size: number) => void
}

/// A drag handle pinned to one edge of the pane it sizes. It measures its own
/// parent, so a pane can stay content-sized until someone drags it.
export function Splitter({ axis, dir = 1, value, min, max, label, onChange }: Props) {
  const from = useRef({ pos: 0, size: 0 })
  const limit = (size: number) => Math.min(max(), Math.max(min, size))

  const measure = (node: HTMLElement) => {
    const rect = node.parentElement?.getBoundingClientRect()
    if (!rect) return value ?? min
    return axis === 'x' ? rect.width : rect.height
  }

  return (
    <div
      role="separator"
      aria-orientation={axis === 'x' ? 'vertical' : 'horizontal'}
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={Math.round(max())}
      aria-valuenow={value === null ? undefined : Math.round(value)}
      tabIndex={0}
      className={axis === 'x' ? 'splitter splitter-x' : 'splitter splitter-y'}
      onPointerDown={(event) => {
        event.preventDefault()
        event.currentTarget.setPointerCapture(event.pointerId)
        // The shell snaps to the pointer while a handle is held: the pane
        // widths carry a transition for the collapse, which would otherwise
        // trail the drag by a frame for the whole gesture.
        document.documentElement.classList.add('is-resizing')
        from.current = {
          pos: axis === 'x' ? event.clientX : event.clientY,
          size: measure(event.currentTarget),
        }
      }}
      onPointerMove={(event) => {
        if (!event.currentTarget.hasPointerCapture(event.pointerId)) return
        const pos = axis === 'x' ? event.clientX : event.clientY
        onChange(limit(from.current.size + dir * (pos - from.current.pos)))
      }}
      onPointerUp={(event) => {
        event.currentTarget.releasePointerCapture(event.pointerId)
        document.documentElement.classList.remove('is-resizing')
      }}
      onLostPointerCapture={() => document.documentElement.classList.remove('is-resizing')}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 64 : 16
        const back = axis === 'x' ? 'ArrowLeft' : 'ArrowUp'
        const forward = axis === 'x' ? 'ArrowRight' : 'ArrowDown'
        if (event.key !== back && event.key !== forward) return
        event.preventDefault()
        const base = value === null ? measure(event.currentTarget) : value
        onChange(limit(base + dir * (event.key === forward ? step : -step)))
      }}
    />
  )
}
