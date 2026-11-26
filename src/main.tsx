import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { isTauri } from '@tauri-apps/api/core'

import App from './App'
import { AgentWidget } from './components/AgentWidget'
import { Launcher } from './components/Launcher'
import './styles.css'

type Surface = 'main' | 'launcher' | 'widget'

/// Which surface this window is.
///
/// The URL hash is the first choice, because Rust can put it in the window's
/// address without teaching the page anything new. The window label is the
/// fallback: a window created from a plain asset path still lands on the right
/// view instead of booting the whole app inside a 300 px frame.
async function surface(): Promise<Surface> {
  const hash = window.location.hash.replace(/^#\/?/, '')
  if (hash === 'launcher' || hash === 'widget') return hash
  if (!isTauri()) return 'main'
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window')
    const label = getCurrentWindow().label
    return label === 'launcher' || label === 'widget' ? label : 'main'
  } catch {
    return 'main'
  }
}

const root = document.getElementById('root')
if (!root) throw new Error('index.html is missing #root')

const surfaceName = await surface()

createRoot(root).render(
  <StrictMode>
    {surfaceName === 'launcher' ? (
      <Launcher />
    ) : surfaceName === 'widget' ? (
      <AgentWidget />
    ) : isTauri() ? (
      <App />
    ) : (
      <main className="empty desktop-required">
        <h2>Open Nova Desktop</h2>
        <p>Launch the Nova Desktop application to use your workspace, agents, and terminal.</p>
      </main>
    )}
  </StrictMode>,
)

/// The boot curtain in index.html lifts on evidence, not on a timer: two frames
/// means React has painted the first screen, and the fonts promise is what keeps
/// the wordmark from swapping type in front of the user. The timeout is only for
/// a font host that never answers - a stuck load must not hold the window.
const boot = document.getElementById('boot')
if (boot && surfaceName !== 'main') {
  // A launcher or widget frame has no boot screen to wait for.
  boot.remove()
} else if (boot) {
  const painted = new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
  })
  const type = document.fonts?.ready ?? Promise.resolve()
  const patience = new Promise<void>((resolve) => setTimeout(resolve, 1500))
  void Promise.all([painted, Promise.race([type, patience])]).then(() => {
    boot.dataset.done = 'true'
    setTimeout(() => boot.remove(), 380)
  })
}
