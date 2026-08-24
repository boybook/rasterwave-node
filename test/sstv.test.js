'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { SstvDecoder, SstvMode, sstvModes } = require('..')
const { encodeSstv, pushAll } = require('./helpers')

function pattern(width, height, color) {
  const pixels = new Uint8Array(width * height * 3)
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const offset = (y * width + x) * 3
      if (color) {
        pixels[offset] = Math.round(x * 255 / Math.max(width - 1, 1))
        pixels[offset + 1] = Math.round(y * 255 / Math.max(height - 1, 1))
        pixels[offset + 2] = (x + y) % 256
      } else {
        const value = Math.round(x * 255 / Math.max(width - 1, 1))
        pixels[offset] = value; pixels[offset + 1] = value; pixels[offset + 2] = value
      }
    }
  }
  return pixels
}

for (const [mode, color] of [[SstvMode.Robot8Bw, false], [SstvMode.Robot36, true]]) {
  test(`streams auto-detected ${mode} rows before EOF`, async () => {
    const info = sstvModes().find(item => item.mode === mode)
    const audio = await encodeSstv(pattern(info.width, info.height, color), mode)
    const events = []
    const decoder = new SstvDecoder(12000, null, event => events.push(event))
    await pushAll(decoder, audio)
    await decoder.finish()
    const started = events.find(event => event.type === 'imageStarted')
    const firstLine = events.findIndex(event => event.type === 'lineReady')
    const finished = events.findIndex(event => event.type === 'finished')
    assert.equal(started.mode, mode)
    assert.ok(firstLine >= 0 && firstLine < finished)
    assert.equal(events.filter(event => event.type === 'lineReady' && event.completeness === 'final').length, info.height)
    assert.equal(events.find(event => event.type === 'imageCompleted').lines, info.height)
    if (mode === SstvMode.Robot36) {
      assert.ok(events.some(event => event.type === 'lineReady' && event.revision > 0))
    }
    await decoder.dispose()
  })
}

test('all modes construct and produce a first asynchronous PCM chunk', async () => {
  for (const info of sstvModes()) {
    const pixels = new Uint8Array(info.width * info.height * 3)
    const { SstvEncoder } = require('..')
    const encoder = new SstvEncoder(pixels, info.mode, 12000, null)
    assert.equal((await encoder.readSamples(128)).length, 128, info.mode)
    await encoder.dispose()
  }
})

test('immediateDecode starts a fixed mode and emits rows without VIS or sync', async () => {
  const info = sstvModes().find(item => item.mode === SstvMode.Robot36)
  const events = []
  const decoder = new SstvDecoder(12000, {
    immediateDecode: true,
    detectVis: false,
    detectSyncTiming: false,
    manualMode: SstvMode.Robot36,
    minimumSignalLevel: 1,
  }, event => events.push(event))
  const silence = new Float32Array(Math.ceil(info.lineSeconds * 12000) + 2)

  await pushAll(decoder, silence)
  await decoder.drain()

  assert.equal(events[0].type, 'imageStarted')
  assert.equal(events[0].mode, SstvMode.Robot36)
  assert.equal(events[0].detection, 'manual')
  assert.ok(events.some(event => event.type === 'lineReady'))
  await decoder.dispose()
})
