import { useSyncExternalStore } from 'react'
import { loadPreferences } from '../lib/settings'
import { pauseSpeaking, playbackSnapshot, resumeSpeaking, speakReply, stopSpeaking, subscribePlayback } from '../lib/speak'

export function Playback({ id, text }: { id?: string; text?: string }) {
  const state = useSyncExternalStore(subscribePlayback, playbackSnapshot)
  const active = Boolean(state.id) && (!id || state.id === id)
  if (!text && (!active || state.status === 'idle')) return null
  return <span className="playback" aria-label="Read aloud controls">
    {text && (!active || state.status === 'idle' || state.status === 'failed') && <button type="button" className="ghost"
      onClick={() => void speakReply(text, { ...loadPreferences(), speak:true }, id)}>Read aloud</button>}
    {active && state.status !== 'idle' && state.status !== 'failed' && <>
      <button type="button" className="ghost" onClick={state.status === 'paused' ? resumeSpeaking : pauseSpeaking}>
        {state.status === 'paused' ? 'Resume' : state.status === 'loading' ? 'Pause loading audio' : 'Pause audio'}
      </button>
      <button type="button" className="ghost" onClick={stopSpeaking}>Stop audio</button>
    </>}
    {active && state.status === 'failed' && <small role="status">{state.error}</small>}
  </span>
}
