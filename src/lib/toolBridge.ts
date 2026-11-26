import { listen } from '@tauri-apps/api/event'

import { errorText } from './ipc'
import { toolReply } from './tools'

/// One tool call from Rust, waiting for the window to answer.
export type BridgeRequest = { id: string; name: string; args: Record<string, unknown> }

export type ToolHandler = (
  name: string,
  args: Record<string, unknown>,
) => unknown | Promise<unknown>

/// Listens for the task tools Rust cannot answer itself.
///
/// The handler is passed as a getter so the listener can be registered once, at
/// mount, while every call still sees the current render's chats: a stored
/// closure would answer from the task list as it was the first time the page
/// rendered, which is exactly the drift this bridge exists to avoid.
export async function startToolBridge(handler: () => ToolHandler): Promise<() => void> {
  const off = await listen<BridgeRequest>('nova:tool-call', ({ payload }) => {
    const id = payload?.id
    const name = payload?.name
    if (typeof id !== 'string' || typeof name !== 'string') return
    let answer: unknown
    try {
      answer = handler()(name, payload.args ?? {})
    } catch (err) {
      void toolReply(id, { ok: false, error: errorText(err) }).catch(() => {})
      return
    }
    void Promise.resolve(answer)
      .then((value) => toolReply(id, { ok: true, value: value ?? null }))
      .catch((err) => toolReply(id, { ok: false, error: errorText(err) }))
      .catch(() => {})
  })
  return off
}
