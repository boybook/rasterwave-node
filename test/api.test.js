'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const rasterwave = require('..')

test('exports the complete SSTV mode catalog', () => {
  const modes = rasterwave.sstvModes()
  assert.equal(modes.length, 31)
  assert.equal(new Set(modes.map(mode => mode.mode)).size, 31)
  assert.equal(rasterwave.SstvMode.Robot36, 'robot36')
  assert.equal(rasterwave.FaxIoc.Ioc288, 'ioc288')
})

test('ESM named imports resolve through the CommonJS loader', async () => {
  const imported = await import('../index.js')
  assert.equal(imported.SstvDecoder, rasterwave.SstvDecoder)
  assert.equal(imported.SstvMode.Robot8Bw, 'robot8Bw')
})

test('encoder reads are native promises and preserve FIFO progress', async () => {
  const info = rasterwave.sstvModes().find(mode => mode.mode === 'robot8Bw')
  const pixels = new Uint8Array(info.width * info.height * 3)
  const encoder = new rasterwave.SstvEncoder(pixels, info.mode, 12000, null)
  const completionOrder = []
  const reads = [0, 1, 2].map(index => encoder.readSamples(2048).then(chunk => {
    completionOrder.push(index)
    return chunk
  }))
  assert.ok(reads.every(value => value instanceof Promise))
  const chunks = await Promise.all(reads)
  assert.deepEqual(chunks.map(chunk => chunk.length), [2048, 2048, 2048])
  assert.deepEqual(completionOrder, [0, 1, 2])
  assert.equal(encoder.progress.samplesEmitted, 6144)
  await encoder.dispose()
  assert.throws(() => encoder.readSamples(1), /RASTERWAVE_DISPOSED/)
})

test('large native reads do not block the Node event loop', async () => {
  const info = rasterwave.sstvModes().find(mode => mode.mode === 'pd290')
  const pixels = new Uint8Array(info.width * info.height * 3)
  const encoder = new rasterwave.SstvEncoder(pixels, info.mode, 12000, null)
  let immediateRan = false
  const read = encoder.readSamples(1_048_576)
  setImmediate(() => { immediateRan = true })
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(immediateRan, true)
  assert.ok((await read).length > 0)
  await encoder.dispose()
})

test('dispose cancels encoder work that has not started', async () => {
  const info = rasterwave.sstvModes().find(mode => mode.mode === 'pd290')
  const pixels = new Uint8Array(info.width * info.height * 3)
  const encoder = new rasterwave.SstvEncoder(pixels, info.mode, 12000, null)
  const reads = Array.from({ length: 16 }, () => encoder.readSamples(1_048_576))
  const disposal = encoder.dispose()
  const settled = await Promise.allSettled(reads)
  await disposal
  assert.ok(settled.some(result => result.status === 'rejected' && /RASTERWAVE_DISPOSED/.test(result.reason.message)))
})

test('decoder write side is synchronous and provides bounded backpressure', async () => {
  const events = []
  const decoder = new rasterwave.SstvDecoder(12000, { queueCapacitySamples: 60000 }, event => events.push(event))
  const chunk = new Float32Array(60000)
  assert.equal(decoder.pushF32(chunk), true)
  assert.equal(decoder.pushF32(chunk), false)
  const barrier = decoder.drain()
  assert.ok(barrier instanceof Promise)
  await barrier
  assert.equal(decoder.queuedSamples, 0)
  assert.equal(events.at(-1).type, 'drain')
  assert.throws(() => decoder.pushF32(new Float32Array(60001)), /RASTERWAVE_CHUNK_TOO_LARGE/)
  await decoder.dispose()
})

test('callback exceptions reject barriers instead of escaping as uncaught exceptions', async () => {
  let rejectNextDrain = true
  const decoder = new rasterwave.SstvDecoder(12000, null, event => {
    if (event.type === 'drain' && rejectNextDrain) {
      rejectNextDrain = false
      throw new Error('callback boom')
    }
  })
  await assert.rejects(decoder.drain(), /RASTERWAVE_CALLBACK_FAILED/)
  await decoder.dispose()
})
