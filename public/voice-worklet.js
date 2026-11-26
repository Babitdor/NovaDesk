// Microphone tap for dictation.
//
// Loaded as a same-origin module (public/voice-worklet.js) rather than a blob:
// URL, because the app's CSP allows scripts only from 'self'. Every render
// quantum posts its mono frames back to the page, which keeps the recorded
// samples exact: no container, no codec, nothing to decode afterwards.
class NovaRecorder extends AudioWorkletProcessor {
  process(inputs) {
    const channel = inputs[0] && inputs[0][0]
    if (channel && channel.length) this.port.postMessage(new Float32Array(channel))
    return true
  }
}

registerProcessor('nova-recorder', NovaRecorder)
