import type { ReactNode } from 'react'

import type { AgentId } from '../lib/settings'

/// One glyph per harness, drawn here rather than borrowed: the real vendor logos
/// are trademarks (and several harnesses publish none), so a harness gets a mark
/// that reads at 15px and belongs to this app instead. They are labels, not
/// claims of affiliation — the accessible name always comes from the text next to
/// them, which is why every mark is aria-hidden.
const GLYPHS: Record<AgentId, ReactNode> = {
  // The app's own four-point spark.
  nova: <path d="M12 2.6 13.9 9.6 21.4 12 13.9 14.4 12 21.4 10.1 14.4 2.6 12 10.1 9.6Z" />,
  // Pi: the letter the harness is named after.
  pi: <path d="M5 7.5h14M9.5 7.5v10M14.5 7.5v10" />,
  // Claude: a code bracket arc with the prompt dot inside it.
  claude: <><path d="M16.9 7.9A6.4 6.4 0 1 0 16.9 16.1" /><circle cx="13.9" cy="12" r="1.4" /></>,
  // Codex: a shell prompt — the caret and the cursor it leaves behind.
  codex: <path d="M6.6 6.2 12.4 12l-5.8 5.8M13.8 18.2h4.8" />,
  // OpenCode: two open walls, the bracket left unclosed.
  opencode: <path d="M9 4.5H4.5v15H9M15 4.5h4.5v15H15" />,
  // CommandCode: the command key, four loops around its centre.
  commandcode: <>
    <path d="M9.2 9.2h5.6v5.6H9.2z" />
    <path d="M9.2 9.2V7.4a2.2 2.2 0 1 0-2.2 2.2h2.2M14.8 9.2h2.2a2.2 2.2 0 1 0-2.2-2.2v2.2M14.8 14.8v2.2a2.2 2.2 0 1 0 2.2-2.2h-2.2M9.2 14.8H7a2.2 2.2 0 1 0 2.2 2.2v-2.2" />
  </>,
  // Hermes: the messenger's folded note in flight.
  hermes: <><path d="M3.4 12.2 20.6 4.9l-4.4 15.4-4.2-6.1z" /><path d="M12 14.2 20.6 4.9" /></>,
}

type Props = {
  id: AgentId
  /// The framed chip used in a settings row; otherwise the bare glyph.
  chip?: boolean
  size?: number
}

export function AgentMark({ id, chip = false, size }: Props) {
  return (
    <span className="agent-mark" data-chip={chip}>
      <svg className="agent-mark-glyph" viewBox="0 0 24 24" width={size} height={size}
        aria-hidden="true" focusable="false">
        {GLYPHS[id]}
      </svg>
    </span>
  )
}
