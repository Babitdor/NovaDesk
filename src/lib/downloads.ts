// Downloads that outlive the pane that started them.
//
// A transfer belongs to Rust from the first byte: the child keeps writing
// whether or not the settings pane is mounted. The progress map has to live at
// the same scope, or switching tab unmounts the only copy of it — the row
// forgets a download that is still running, offers Download again, and a second
// click starts a second writer into the same folder.
//
// So the map lives here, keyed by model id, and a pane subscribes to it. The
// invoke lives here too: the promise must not depend on a component staying
// mounted.
export type Downloads<T> = {
  subscribe: (listener: () => void) => () => void
  /// The current map, replaced rather than mutated: React needs a new identity
  /// to see the change.
  snapshot: () => Map<string, T>
  /// Starts one transfer and resolves when it ends, so a pane that is still
  /// mounted can report the outcome. An unmounted pane simply stops caring.
  start: (id: string, first: T) => Promise<void>
}

export function createDownloads<T>(
  transfer: (id: string, onEvent: (frame: T) => void) => Promise<void>,
): Downloads<T> {
  let frames = new Map<string, T>()
  const listeners = new Set<() => void>()
  const publish = (next: Map<string, T>) => {
    frames = next
    for (const listener of listeners) listener()
  }
  return {
    subscribe: (listener) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    snapshot: () => frames,
    start: (id, first) => {
      publish(new Map(frames).set(id, first))
      return transfer(id, (frame) => publish(new Map(frames).set(id, frame)))
        .finally(() => {
          const next = new Map(frames)
          next.delete(id)
          publish(next)
        })
    },
  }
}

