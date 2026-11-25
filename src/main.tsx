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
