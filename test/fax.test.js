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

test('immediateDecode starts a fixed profile and emits rows without APT', async () => {
  const events = []
  const decoder = new FaxDecoder(12000, {
    immediateDecode: true,
    ioc: FaxIoc.Ioc576,
    lpm: 120,
    minimumSignalLevel: 1,
    minimumCarrierCoherence: 2,
  }, event => events.push(event))

  await pushAll(decoder, new Float32Array(6002))
  await decoder.drain()

  assert.equal(events[0].type, 'pageStarted')
  assert.equal(events[0].ioc, FaxIoc.Ioc576)
  assert.equal(events[0].lpm, 120)
  assert.ok(events.some(event => event.type === 'lineReady'))
  assert.ok(!events.some(event => event.type === 'aptDetected'))
  await decoder.dispose()
})

test('continuousPaper fax prints without APT and signal loss only inserts a boundary', async () => {
  const events = []
  const decoder = new FaxDecoder(12000, {
    outputMode: 'continuousPaper',
    continuousAuto: false,
    ioc: FaxIoc.Ioc576,
    lpm: 120,
    minimumSignalLevel: 0,
    minimumCarrierCoherence: 0,
  }, event => events.push(event))

  assert.equal(decoder.pushF32(new Float32Array(6002)), true)
  assert.ok(decoder.drain() instanceof Promise)
  await decoder.drain()
  assert.equal(decoder.markSignalLost(), true)
  await decoder.drain()

  assert.equal(events[0].type, 'paperStarted')
  assert.ok(events.some(event => event.type === 'rasterLineReady'))
  assert.ok(events.some(event => event.type === 'rasterBoundary' && event.boundaryKind === 'discontinuity'))
  assert.ok(!events.some(event => event.type === 'transmissionCompleted'))
  await decoder.dispose()
})

test('continuousPaper fax self-decodes APT start and stop', async () => {
  const width = 864
  const height = 4
  const spec = {
    ioc: FaxIoc.Ioc288,
    lpm: 240,
    phasingSeconds: 3,
    startSeconds: 1,
    stopSeconds: 2,
    trailingBlackSeconds: 0.1,
  }
  const audio = await encodeFax(faxPattern(width, height), width, height, spec)
  const events = []
  const decoder = new FaxDecoder(12000, {
    outputMode: 'continuousPaper',
    continuousAuto: true,
    ioc: FaxIoc.Ioc288,
    lpm: 240,
    expectedPhasingSeconds: 3,
    aptConfirmSeconds: 0.5,
    acquisitionTimeoutSeconds: 10,
    stopConfirmSeconds: 0.5,
    signalLossSeconds: 3,
    minimumCarrierCoherence: 0,
  }, event => events.push(event))
  await pushAll(decoder, audio)
  await decoder.drain()

  const boundary = events.find(event => event.type === 'rasterBoundary' && event.boundaryKind === 'aptPhasing' && event.trusted)
  const completed = events.find(event => event.type === 'transmissionCompleted')
  assert.equal(boundary.ioc, FaxIoc.Ioc288)
  assert.equal(completed.boundaryId, boundary.boundaryId)
  assert.equal(completed.endLine - completed.startLine, completed.lines)
  await decoder.dispose()
})
