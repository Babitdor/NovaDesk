export type OutputStream = 'stdout' | 'stderr'

export type FinishReason = 'completed' | 'cancelled' | 'failed'

/// Mirrors `AgentEvent` in src-tauri/src/events.rs. Events carry no turn id: each
/// turn owns its own channel, so the caller already knows which turn it is on.
export type AgentEvent =
  | { type: 'turnStarted' }
  | { type:'timing'; phase:string; ms:number }
  | { type: 'approvalResolved'; interruptId: string }
  | { type: 'session'; id: string }
  | { type: 'usage'; input: number; output: number }
  | { type: 'reasoningDelta'; text: string }
  | { type: 'messageDelta'; text: string }
  | { type: 'finalAnswerDelta'; text: string }
  | { type: 'messagePreview'; text: string }
  | { type: 'messageFinal'; text: string; author?: string | null; replacePreview: boolean; plain: boolean }
  | { type: 'textDiscard' }
  | { type: 'toolCall'; callId: string; name: string; input: string }
  | { type: 'toolResult'; callId: string; ok: boolean; output: string }
  | { type: 'fileEdit'; path: string; diff: string }
  | { type: 'subagentTask'; taskId: string; status: string; label?: string | null; subagentType?: string | null; durationMs?: number | null }
  | { type: 'subagentPreview'; taskId: string; text: string }
  | { type: 'approvalRequired'; interruptId: string; kind: string; payload: unknown }
  | { type: 'notice'; level: string; message: string }
  | { type: 'processOutput'; stream: OutputStream; chunk: string }
  | { type: 'turnError'; message: string; kind?: string }
  | { type: 'turnFinished'; reason: FinishReason }
  /// The harness asked Nova to start a new task later. Requested through
  /// `NOVA_DESKTOP_REQUESTS`, never through a command.
  | { type: 'wakeupRequested'; prompt: string; agent?: string | null; afterSeconds?: number | null }
  /// The harness asked Nova to put a message in another task.
  | { type: 'threadMessage'; thread: string; text: string }

export type WorkspaceInfo = { root: string; name: string }

export type DirEntry = { path: string; name: string; isDir: boolean }

export type InterruptReply =
  | { kind: 'tool'; approve: boolean }
  | { kind: 'plan'; approve: boolean; feedback?: string }
  | { kind: 'question'; answer: string; selectedIndex: number | null }
