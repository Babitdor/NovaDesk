import { useSyncExternalStore } from 'react'
import { loadPreferences } from '../lib/settings'
import { pauseSpeaking, playbackSnapshot, resumeSpeaking, speakReply, stopSpeaking, subscribePlayback } from '../lib/speak'

/// Icons instead of three words: the reply head already carries "Copy reply", and
/// stacked labels there read as a sentence rather than controls. Each button keeps
/// a title and an aria-label, so the drawing is never the only name for it.
export function Playback({ id, text }: { id?: string; text?: string }) {
  const state = useSyncExternalStore(subscribePlayback, playbackSnapshot)
  const active = Boolean(state.id) && (!id || state.id === id)
  if (!text && (!active || state.status === 'idle')) return null
  const playing = active && state.status !== 'idle' && state.status !== 'failed'
  const pauseLabel = state.status === 'paused' ? 'Resume audio'
    : state.status === 'loading' ? 'Pause loading audio' : 'Pause audio'
  return <span className="playback" aria-label="Read aloud controls">
    {text && !playing && <button type="button" className="ghost" title="Read aloud" aria-label="Read aloud"
      onClick={() => void speakReply(text, { ...loadPreferences(), speak:true }, id)}>
      <SpeakGlyph />
    </button>}
    {playing && <>
      <button type="button" className="ghost" title={pauseLabel} aria-label={pauseLabel}
        onClick={state.status === 'paused' ? resumeSpeaking : pauseSpeaking}>
        {state.status === 'paused' ? <ResumeGlyph /> : <PauseGlyph />}
      </button>
      <button type="button" className="ghost" title="Stop audio" aria-label="Stop audio" onClick={stopSpeaking}>
        <StopGlyph />
      </button>
    </>}
    {active && state.status === 'failed' && <small role="status">{state.error}</small>}
  </span>
}

/// A speaker with its waves: the reply is read out.
function SpeakGlyph() {
  return <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
    <path d="M2.6 6.4h2.1L7.3 4v8L4.7 9.6H2.6z" fill="none" stroke="currentColor" strokeWidth="1.4" />
    <path d="M9.4 6.2c.9.9.9 2.7 0 3.6M11.6 4.6c1.8 1.8 1.8 5 0 6.8"
      fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
  </svg>
}

/// Two bars: the audio is running and can be held.
function PauseGlyph() {
  return <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
    <path d="M6 4.4v7.2M10 4.4v7.2" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />
  </svg>
}

/// The pause glyph's own inverse, so the same button reads as "carry on".
function ResumeGlyph() {
  return <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
    <path d="M6.2 4.3 11.4 8l-5.2 3.7z" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
  </svg>
}

/// A speaker struck through: nothing more is read out.
function StopGlyph() {
  return <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
    <path d="M2.4 6.3h2.1L7 3.9v8.2L4.5 9.7H2.4z" fill="none" stroke="currentColor" strokeWidth="1.4" />
    <path d="M9.7 6.3l3.5 3.4M13.2 6.3 9.7 9.7"
      stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
  </svg>
}
