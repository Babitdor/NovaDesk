import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { isTauri } from '@tauri-apps/api/core'

import App from './App'
import './styles.css'

const root = document.getElementById('root')
if (!root) throw new Error('index.html is missing #root')

createRoot(root).render(
  <StrictMode>
    {isTauri() ? (
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
if (boot) {
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
