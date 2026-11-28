import type { KeyboardEvent } from 'react'

export function navigateComposerMenu(event: KeyboardEvent<HTMLDivElement>) {
  const menu = event.currentTarget
  if (event.key === 'Tab') { menu.hidePopover(); return }
  const items = Array.from(menu.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"], [role="menuitem"]'))
  const current = items.indexOf(document.activeElement as HTMLButtonElement)
  const next = event.key === 'ArrowDown' ? (current + 1) % items.length
    : event.key === 'ArrowUp' ? (current + items.length - 1) % items.length
    : event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : -1
  if (next >= 0) { event.preventDefault(); items[next].focus() }
}
