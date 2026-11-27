import assert from 'node:assert/strict'
import { test } from 'node:test'
import { SpeechSentences } from './speechQueue.ts'

test('only final sentences stream, and the final replacement is deduplicated', () => {
  const queue = new SpeechSentences()
  assert.deepEqual(queue.accept({ type:'messagePreview', text:'Private thought. ' }), [])
  assert.deepEqual(queue.accept({ type:'messageDelta', text:'Provisional. ' }), [])
  assert.deepEqual(queue.accept({ type:'finalAnswerDelta', text:'First sentence. More' }), ['First sentence.'])
  assert.deepEqual(queue.accept({ type:'finalAnswerDelta', text:' words' }), [])
  queue.accept({ type:'messageFinal', text:'First sentence. More words', replacePreview:true, plain:false })
  assert.deepEqual(queue.accept({ type:'turnFinished', reason:'completed' }), ['More words'])
  assert.deepEqual(queue.accept({ type:'turnFinished', reason:'completed' }), [])
})

test('fallback speaks only the last user-facing answer at completion', () => {
  const queue = new SpeechSentences()
  queue.accept({ type:'messageFinal', text:'Provisional plan.', plain:false, replacePreview:true })
  queue.accept({ type:'messageFinal', text:'Teammate message.', author:'Nora', plain:false, replacePreview:false })
  queue.accept({ type:'messageFinal', text:'Done. ```code``` Remaining 1.5 seconds.', plain:false, replacePreview:true })
  assert.deepEqual(queue.accept({ type:'turnFinished', reason:'completed' }), ['Done.', 'Remaining 1.5 seconds.'])
})

test('cancellation and plain reports produce no speech', () => {
  const queue = new SpeechSentences()
  queue.accept({ type:'messageFinal', text:'tool output', plain:true, replacePreview:false })
  assert.deepEqual(queue.accept({ type:'turnFinished', reason:'cancelled' }), [])
})
