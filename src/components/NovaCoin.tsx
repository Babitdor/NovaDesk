import { useRef } from 'react'

/// The body is the same circle struck a pixel apart through the thickness.
/// A single flat disc would thin to a line at the quarter turn; a stack keeps
/// a rim on the coin wherever the turn leaves it.
const SPAN = 6 // stays inside the 7px --coin-edge in styles.css
const BODY = Array.from({ length: 18 }, (_, index) => (index / 17 - 0.5) * SPAN)

/// The standing emblem, earned rather than printed: two faces, a milled body and
/// one slow turn. It turns on its own so the empty plate is not a still frame,
/// and dragging it by hand pauses the idle turn and picks up from the angle the
/// hand left it at — the paused animation resumes in place, so nothing jumps.
export function NovaCoin() {
  const coin = useRef<HTMLDivElement>(null)
  const hand = useRef({ angle: 0, from: 0, dragging: false })

  return (
    <div
      className="coin"
      ref={coin}
      aria-hidden="true"
      onPointerDown={(event) => {
        hand.current.dragging = true
        hand.current.from = event.clientX
        event.currentTarget.setPointerCapture(event.pointerId)
        event.currentTarget.dataset.dragging = 'true'
      }}
      onPointerMove={(event) => {
        if (!hand.current.dragging) return
        // 0.7deg per pixel: a short drag turns the coin far enough to read the
        // rim, without the face snapping away from the pointer.
        hand.current.angle += (event.clientX - hand.current.from) * 0.7
        hand.current.from = event.clientX
        coin.current?.style.setProperty('--coin-turn', `${hand.current.angle}deg`)
      }}
      onPointerUp={(event) => {
        hand.current.dragging = false
        event.currentTarget.releasePointerCapture(event.pointerId)
        delete event.currentTarget.dataset.dragging
      }}
      onPointerCancel={(event) => {
        hand.current.dragging = false
        delete event.currentTarget.dataset.dragging
      }}
    >
      <div className="coin-body">
        <div className="coin-spin">
          <div className="coin-face coin-obverse"><img src="/nova.webp" alt="" draggable={false} /></div>
          <div className="coin-face coin-reverse"><img src="/nova.webp" alt="" draggable={false} /></div>
          {BODY.map((offset) => (
            <span key={offset} className="coin-edge" style={{ transform: `translateZ(${offset}px)` }} />
          ))}
        </div>
      </div>
    </div>
  )
}
