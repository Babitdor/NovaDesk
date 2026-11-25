import type { AgentId } from '../lib/settings'

import claudeMark from '../assets/harness/claude.svg'
import commandcodeMark from '../assets/harness/commandcode.svg'
import hermesMark from '../assets/harness/hermes.png'
import openaiMark from '../assets/harness/openai.svg'
import opencodeMark from '../assets/harness/opencode.svg'
import piMark from '../assets/harness/pi.svg'

/// Each harness shows the mark its vendor publishes — the favicon, brand file or
/// symbol from their own site or repository — so a list of agents is recognisable
/// at a glance instead of reading as seven letters. Nova is the exception: it is
/// the same portrait the coin is struck from, so the app's own agent looks like
/// the app. src/assets/harness/SOURCES.md records where every file came from.
const MARKS: Record<AgentId, string> = {
  nova: '/nova.webp',
  pi: piMark,
  claude: claudeMark,
  codex: openaiMark,
  opencode: opencodeMark,
  commandcode: commandcodeMark,
  hermes: hermesMark,
}

type Props = {
  id: AgentId
  /// The framed chip used in a settings row; otherwise the bare mark.
  chip?: boolean
}

export function AgentMark({ id, chip = false }: Props) {
  return (
    <span className="agent-mark" data-chip={chip}>
      {/* Decorative: the harness name next to it is the label. */}
      <img className="agent-mark-glyph" src={MARKS[id]} alt="" aria-hidden="true" />
    </span>
  )
}
