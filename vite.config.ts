import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // Tauri expects a fixed port and must not clear the Rust log output.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // The Rust crate lives inside the Vite root; watching it causes reload loops.
    watch: { ignored: ['**/src-tauri/**'] },
  },
})
