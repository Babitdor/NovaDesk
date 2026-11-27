import type { AgentEvent } from './events'

/// Only explicitly final deltas can leave this buffer before turn completion.
export class SpeechSentences {
  private text = ''
  private emitted = ''
  private replacement = ''
  private stopped = false

  accept(event: AgentEvent): string[] {
    if (this.stopped) return []
    if (event.type === 'finalAnswerDelta') {
      this.text += event.text
      return this.take(false)
    }
    if (event.type === 'messageFinal' && !event.plain && (!event.author || event.author.toLowerCase() === 'nova')) this.replacement = event.text
    if (event.type !== 'turnFinished') return []
    this.stopped = true
    if (event.reason !== 'completed') return []
    if (this.replacement) this.text = this.replacement
    return this.take(true)
  }

  private take(flush: boolean): string[] {
    const clean = this.text.replace(/```[\s\S]*?(?:```|$)/g, ' ').replace(/\s+/g, ' ').trimStart()
    // A corrected final answer cannot retract audio; never replay its prefix.
    if (!clean.startsWith(this.emitted)) return []
    const rest = clean.slice(this.emitted.length)
    const segments = [...new Intl.Segmenter(undefined, { granularity: 'sentence' }).segment(rest)]
    const result: string[] = []
    for (const { segment, index } of segments) {
      const complete = flush || (index + segment.length < rest.length || /[.!?]["')\]]?\s$/.test(segment))
      if (!complete) break
      this.emitted += segment
      if (segment.trim()) result.push(segment.trim())
    }
    return result
  }
}

export type PlaybackState = { id: string; status: 'idle' | 'loading' | 'playing' | 'paused' | 'failed'; error?: string; firstAudioMs?: number }
