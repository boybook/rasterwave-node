'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { FaxDecoder, FaxIoc } = require('..')
const { encodeFax, pushAll } = require('./helpers')

function faxPattern(width, height) {
  const pixels = new Uint8Array(width * height)
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) pixels[y * width + x] = Math.round(x * 255 / (width - 1))
  }
  return pixels
}

for (const modulation of [
  { kind: 'fm' },
  { kind: 'am', carrierHz: 1800, blackLevel: 1, whiteLevel: 0.04 },
]) {
  test(`streams IOC288 ${modulation.kind.toUpperCase()} fax rows`, async () => {
    const width = 864
    const height = 2
    const isAm = modulation.kind === 'am'
    const spec = {
      ioc: FaxIoc.Ioc288,
      lpm: isAm ? 240 : 120,
      modulation,
      ...(isAm ? {} : {
        phasingSeconds: 1,
        startSeconds: 1,
        stopSeconds: 1,
        trailingBlackSeconds: 0.1,
      }),
    }
    const audio = await encodeFax(faxPattern(width, height), width, height, spec)
    const events = []
    const decoder = new FaxDecoder(12000, {
      ioc: FaxIoc.Ioc288,
      lpm: isAm ? 240 : 120,
      modulation,
      maxLines: height,
      ...(isAm ? {} : {
        expectedPhasingSeconds: 1,
        aptConfirmSeconds: 0.2,
        acquisitionTimeoutSeconds: 5,
        stopConfirmSeconds: 0.2,
        signalLossSeconds: 0.5,
        minimumCarrierCoherence: 0,
      }),
    }, event => events.push(event))
    await pushAll(decoder, audio)
    await decoder.finish()
    assert.equal(events.filter(event => event.type === 'lineReady').length, height)
    assert.ok(events.filter(event => event.type === 'lineReady').every(event => event.pixels.length === 905))
    assert.equal(events.find(event => event.type === 'pageCompleted').partial, false)
    assert.ok(events.findIndex(event => event.type === 'lineReady') < events.findIndex(event => event.type === 'finished'))
    await decoder.dispose()
  })
}

test('markSignalLost closes an active page as partial', async () => {
  const width = 864
  const spec = { ioc: FaxIoc.Ioc288, lpm: 120, phasingSeconds: 1, startSeconds: 1, stopSeconds: 1, trailingBlackSeconds: 0.1 }
  const audio = await encodeFax(faxPattern(width, 2), width, 2, spec)
  const events = []
  const decoder = new FaxDecoder(12000, {
    ioc: FaxIoc.Ioc288, lpm: 120, expectedPhasingSeconds: 1,
    aptConfirmSeconds: 0.2, acquisitionTimeoutSeconds: 5,
    stopConfirmSeconds: 0.2, signalLossSeconds: 2, minimumCarrierCoherence: 0,
  }, event => events.push(event))
  await pushAll(decoder, audio.subarray(0, Math.floor(audio.length * 0.75)))
  assert.equal(decoder.markSignalLost(), true)
  await decoder.drain()
  assert.equal(events.find(event => event.type === 'pageCompleted').partial, true)
  await decoder.dispose()
})
