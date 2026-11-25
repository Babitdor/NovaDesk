import { useEffect, useRef } from 'react'

/// The night sky is a place, not a texture: a tiled radial-gradient cannot twinkle,
/// cannot be pushed, and cannot send anything across the frame. This paints three
/// depth layers of stars on a canvas, drifts them against the pointer, lights the
/// ones the hand comes near, and lets a meteor cross every few seconds.
///
/// It is also the most expensive thing on the empty screen, so it is a canvas with a
/// capped pixel ratio, ~300 stars at 1440x900, one rAF loop that stops when the
/// window is hidden, and a single static frame when motion is not wanted.

type Star = {
  x: number
  y: number
  /// 0 = far (drifts least), 1 = near (drifts most, brightest).
  depth: number
  size: number
  phase: number
  pulse: number
  warm: boolean
}

type Meteor = { x: number; y: number; vx: number; vy: number; ttl: number; life: number; length: number }
type Spark = { x: number; y: number; vx: number; vy: number; ttl: number; life: number }

const DENSITY = 1 / 2600 // stars per CSS pixel of sky
const MIN_STARS = 160
const MAX_STARS = 520
const POINTER_RADIUS = 150

function buildStars(width: number, height: number): Star[] {
  const count = Math.round(Math.min(MAX_STARS, Math.max(MIN_STARS, width * height * DENSITY)))
  const stars: Star[] = []
  for (let index = 0; index < count; index += 1) {
    // Square the depth so most stars sit far away: an even spread reads as noise.
    const depth = Math.random() ** 2
    stars.push({
      x: Math.random() * width,
      y: Math.random() * height,
      depth,
      size: 0.45 + depth * 1.35,
      phase: Math.random() * Math.PI * 2,
      pulse: 0.4 + Math.random() * 1.4,
      warm: Math.random() < 0.18,
    })
  }
  return stars
}

export function Sky({ still }: { still: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const element = canvas.current
    const context = element?.getContext('2d')
    if (!element || !context) return

    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    const motion = !still && !reduce

    let width = 0
    let height = 0
    let dpr = 1
    let stars: Star[] = []
    let meteors: Meteor[] = []
    let sparks: Spark[] = []
    let frame = 0
    let streak = 0
    let last = performance.now()
    // Pointer, smoothed: the sky should lean, not snap.
    const pointer = { x: -1e4, y: -1e4, tx: 0, ty: 0, ox: 0, oy: 0 }

    const resize = () => {
      const rect = element.getBoundingClientRect()
      width = Math.max(1, Math.round(rect.width))
      height = Math.max(1, Math.round(rect.height))
      dpr = Math.min(window.devicePixelRatio || 1, 2)
      element.width = Math.round(width * dpr)
      element.height = Math.round(height * dpr)
      context.setTransform(dpr, 0, 0, dpr, 0, 0)
      stars = buildStars(width, height)
      paint(0)
    }

    const spawnMeteor = (fromX?: number, fromY?: number) => {
      const angle = Math.PI * (0.72 + Math.random() * 0.16) // down-left to down-right
      const speed = 420 + Math.random() * 360
      meteors.push({
        x: fromX ?? width * (0.15 + Math.random() * 0.8),
        y: fromY ?? -20 - Math.random() * height * 0.25,
        vx: Math.cos(angle) * speed,
        vy: Math.sin(angle) * speed * 0.55 + 220,
        ttl: 900 + Math.random() * 700,
        life: 0,
        length: 90 + Math.random() * 120,
      })
    }

    const paint = (dt: number) => {
      context.clearRect(0, 0, width, height)
      if (!motion) {
        // One still frame: every star at its own brightness, no pointer, no meteors.
        for (const star of stars) {
          context.globalAlpha = 0.35 + star.depth * 0.65
          context.fillStyle = star.warm ? '#ffd9b0' : '#e9f0ff'
          context.beginPath()
          context.arc(star.x, star.y, star.size, 0, Math.PI * 2)
          context.fill()
        }
        context.globalAlpha = 1
        return
      }

      const time = performance.now() / 1000
      pointer.ox += (pointer.tx - pointer.ox) * 0.06
      pointer.oy += (pointer.ty - pointer.oy) * 0.06

      for (const star of stars) {
        const drift = (star.depth - 0.35) * 10
        let x = star.x + pointer.ox * star.depth + drift
        let y = star.y + pointer.oy * star.depth + drift * 0.4
        const dx = x - pointer.x
        const dy = y - pointer.y
        const near = Math.hypot(dx, dy)
        let glow = 0
        if (near < POINTER_RADIUS) {
          // The hand wakes the sky: nearby stars brighten and lean away from it.
          const pull = 1 - near / POINTER_RADIUS
          glow = pull * 0.85
          x += (dx / (near || 1)) * pull * 14
          y += (dy / (near || 1)) * pull * 14
        }
        const twinkle = 0.55 + 0.45 * Math.sin(time * star.pulse + star.phase)
        const alpha = Math.min(1, (0.4 + star.depth * 0.7) * twinkle + glow)
        context.globalAlpha = alpha
        context.fillStyle = star.warm ? '#ffd9b0' : '#e9f0ff'
        context.beginPath()
        context.arc(x, y, star.size * (1 + glow * 0.7), 0, Math.PI * 2)
        context.fill()
      }

      // Meteors: a short head with a tail that fades to nothing behind it.
      for (const meteor of meteors) {
        meteor.life += dt
        meteor.x += meteor.vx * dt * 0.001
        meteor.y += meteor.vy * dt * 0.001
        const fade = 1 - meteor.life / meteor.ttl
        const tailX = meteor.x - meteor.vx * 0.12
        const tailY = meteor.y - meteor.vy * 0.12
        const gradient = context.createLinearGradient(meteor.x, meteor.y, tailX, tailY)
        gradient.addColorStop(0, `rgba(255, 244, 220, ${0.9 * fade})`)
        gradient.addColorStop(1, 'rgba(255, 244, 220, 0)')
        context.globalAlpha = 1
        context.strokeStyle = gradient
        context.lineWidth = 1.4
        context.beginPath()
        context.moveTo(meteor.x, meteor.y)
        context.lineTo(tailX, tailY)
        context.stroke()
        context.fillStyle = `rgba(255, 250, 235, ${fade})`
        context.beginPath()
        context.arc(meteor.x, meteor.y, 1.2, 0, Math.PI * 2)
        context.fill()
      }
      meteors = meteors.filter((meteor) => meteor.life < meteor.ttl && meteor.y < height + 60)

      for (const spark of sparks) {
        spark.life += dt
        spark.x += spark.vx * dt * 0.001
        spark.y += spark.vy * dt * 0.001
        spark.vy += 26 * dt * 0.001
        const fade = 1 - spark.life / spark.ttl
        context.globalAlpha = Math.max(0, fade)
        context.fillStyle = '#ffd9b0'
        context.beginPath()
        context.arc(spark.x, spark.y, 0.9, 0, Math.PI * 2)
        context.fill()
      }
      sparks = sparks.filter((spark) => spark.life < spark.ttl)

      context.globalAlpha = 1

      streak += dt
      if (streak > 4200 && Math.random() < 0.02) {
        spawnMeteor()
        streak = 0
      }
    }

    const loop = (now: number) => {
      const dt = Math.min(64, now - last)
      last = now
      paint(dt)
      frame = requestAnimationFrame(loop)
    }

    const start = () => {
      cancelAnimationFrame(frame)
      last = performance.now()
      frame = requestAnimationFrame(loop)
    }

    const onPointerMove = (event: PointerEvent) => {
      const rect = element.getBoundingClientRect()
      pointer.x = event.clientX - rect.left
      pointer.y = event.clientY - rect.top
      pointer.tx = (pointer.x / Math.max(1, rect.width) - 0.5) * -26
      pointer.ty = (pointer.y / Math.max(1, rect.height) - 0.5) * -26
    }

    const onPointerLeave = () => {
      pointer.x = -1e4
      pointer.y = -1e4
      pointer.tx = 0
      pointer.ty = 0
    }

    const onPointerDown = (event: PointerEvent) => {
      const rect = element.getBoundingClientRect()
      const x = event.clientX - rect.left
      const y = event.clientY - rect.top
      for (let index = 0; index < 14; index += 1) {
        const angle = (index / 14) * Math.PI * 2 + Math.random()
        const speed = 40 + Math.random() * 90
        sparks.push({ x, y, vx: Math.cos(angle) * speed, vy: Math.sin(angle) * speed, ttl: 700, life: 0 })
      }
      spawnMeteor(x + 60, y - 40)
    }

    const onVisibility = () => {
      if (document.hidden) cancelAnimationFrame(frame)
      else if (motion) start()
    }

    const observer = new ResizeObserver(resize)
    observer.observe(element)
    resize()

    if (motion) {
      window.addEventListener('pointermove', onPointerMove, { passive: true })
      window.addEventListener('pointerdown', onPointerDown, { passive: true })
      element.addEventListener('pointerleave', onPointerLeave)
      document.addEventListener('visibilitychange', onVisibility)
      start()
    }

    return () => {
      cancelAnimationFrame(frame)
      observer.disconnect()
      window.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('pointerdown', onPointerDown)
      element.removeEventListener('pointerleave', onPointerLeave)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [still])

  return <canvas className="sky" ref={canvas} aria-hidden="true" />
}
