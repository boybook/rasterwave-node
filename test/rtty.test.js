'use strict'

const assert = require('node:assert/strict')
const test = require('node:test')

const { RttyDecoder, RttyEncoder, RttyScanner, rttyProfiles } = require('..')
const { pushAll } = require('./helpers')

function encodeText(text, sampleRate = 12000, options = null) {
  const encoder = new RttyEncoder(sampleRate, options)
  encoder.pushText(text)
  encoder.finish()
  const chunks = []
  const rendered = []
  while (!encoder.isFinished) {
    const result = encoder.readSamples(1200)
    chunks.push(result.samples)
    rendered.push(...result.rendered)
  }
  const pcm = new Float32Array(chunks.reduce((total, chunk) => total + chunk.length, 0))
  let offset = 0
  for (const chunk of chunks) {
    pcm.set(chunk, offset)
    offset += chunk.length
  }
  return { pcm, rendered }
}

async function decode(pcm, sampleRate = 12000, options = null) {
  const events = []
  const decoder = new RttyDecoder(sampleRate, options, event => events.push(event))
  const padded = new Float32Array(pcm.length + sampleRate)
  padded.set(pcm, sampleRate / 2)
  await pushAll(decoder, padded)
  await decoder.finish()
  await decoder.dispose()
  return events
}

const textOf = events => events.filter(e => e.type === 'character').map(e => e.ch).join('').replace(/\r/g, '')

test('exports the RTTY profile catalog', () => {
  const profiles = rttyProfiles()
  assert.deepEqual(profiles.map(p => p.id), ['45/170', '50/170', '75/170', '50/425', '50/850'])
  assert.equal(profiles[0].stopBits, 1.5)
})

test('encoder output decodes back to the same text through the native queue', async () => {
  const text = 'CQ CQ DE BG5DRB BG5DRB K\nRYRY 599 TU 73\n'
  const { pcm, rendered } = encodeText(text)
  assert.equal(rendered.map(r => r.ch).join(''), text)
  assert.ok(rendered.every((r, i) => i === 0 || r.startSample >= rendered[i - 1].endSample))
  const events = await decode(pcm)
  assert.equal(textOf(events).trim(), text.trim())
  assert.ok(events.some(e => e.type === 'signal' && e.locked))
})

test('48 kHz custom tones round trip', async () => {
  const options = { markHz: 1445, spaceHz: 1275 }
  const { pcm } = encodeText('TEST 0123456789 /?-\n', 48000, options)
  const events = await decode(pcm, 48000, options)
  assert.equal(textOf(events).trim(), 'TEST 0123456789 /?-')
})

test('interactive encoder retracts uncommitted text in program order', () => {
  const encoder = new RttyEncoder(12000)
  encoder.readSamples(1200)
  const pushed = encoder.pushText('CQ test%')
  assert.equal(pushed.accepted, 7)
  assert.equal(pushed.firstSeq, 0)
  assert.equal(pushed.rejected, '%')
  assert.equal(encoder.retractUnsent(4), 4)
  assert.equal(encoder.pendingLength, 3)
  assert.equal(encoder.pendingText, 'CQ ')
  const rendered = []
  for (let i = 0; i < 40; i += 1) {
    const result = encoder.readSamples(1200)
    assert.equal(result.samples.length, 1200)
    rendered.push(...result.rendered)
  }
  assert.equal(rendered.map(r => r.ch).join(''), 'CQ ')
  assert.equal(encoder.isFinished, false)
  encoder.abort()
  while (!encoder.isFinished) encoder.readSamples(1200)
  assert.throws(() => encoder.pushText('X'), /RASTERWAVE_ENCODER_CLOSED/)
  encoder.dispose()
  assert.throws(() => encoder.readSamples(1), /RASTERWAVE_DISPOSED/)
})

test('invalid configuration is rejected with stable codes', () => {
  assert.throws(() => new RttyEncoder(12000, { stopBits: 3 }), /RASTERWAVE_INVALID_CONFIG/)
  assert.throws(() => new RttyEncoder(12000, { markHz: 2125 }), /RASTERWAVE_INVALID_CONFIG/)
  assert.throws(() => new RttyDecoder(12000, { lanes: ['bogus'] }, () => {}), /RASTERWAVE_INVALID_CONFIG/)
  const decoder = new RttyDecoder(12000, null, () => {})
  assert.throws(() => decoder.setSquelch(2), /RASTERWAVE_INVALID_ARGUMENT/)
  return decoder.dispose()
})

test('decoder retune and squelch commands are ordered with audio', async () => {
  const { pcm } = encodeText('RYRYRY CQ TEST\n', 12000, { markHz: 1000, spaceHz: 1170 })
  const events = []
  const decoder = new RttyDecoder(12000, null, event => events.push(event))
  assert.equal(decoder.retune(1000, 1170), true)
  assert.equal(decoder.setSquelch(0.25), true)
  await pushAll(decoder, pcm)
  await pushAll(decoder, new Float32Array(6000))
  await decoder.finish()
  await decoder.dispose()
  assert.match(textOf(events), /CQ TEST/)
})

test('settings and discontinuities are ordered with audio', async () => {
  const { pcm } = encodeText('CQ TEST DE N0CALL\n')
  const events = []
  const decoder = new RttyDecoder(12000, { noiseBlanker: true }, event => events.push(event))
  assert.equal(decoder.updateSettings({ afc: false, usos: true, characterSet: 'ita2', lanes: ['atc'], autoReverse: false }), true)
  assert.throws(() => decoder.updateSettings({ lanes: ['nope'] }), /RASTERWAVE_INVALID_ARGUMENT/)
  await pushAll(decoder, pcm)
  assert.equal(decoder.markDiscontinuity(12000), true)
  await decoder.drain()
  assert.match(textOf(events), /CQ TEST DE N0CALL/)
  await decoder.finish()
  await decoder.dispose()
})

test('profile shift without explicit tones selects the matching tone pair', async () => {
  const options = { baud: 50, shiftHz: 850 }
  const { pcm } = encodeText('RYRY WIDE SHIFT\n', 12000, options)
  const events = await decode(pcm, 12000, options)
  assert.equal(textOf(events).trim(), 'RYRY WIDE SHIFT')
})

test('invalid control commands fail synchronously and keep the session usable', async () => {
  const decoder = new RttyDecoder(12000, null, () => {})
  assert.throws(() => decoder.retune(1000, 1005), /RASTERWAVE_INVALID_ARGUMENT/)
  assert.throws(() => decoder.updateSettings({ lanes: [] }), /RASTERWAVE_INVALID_ARGUMENT/)
  assert.equal(decoder.pushF32(new Float32Array(1200)), true)
  assert.equal(decoder.markDiscontinuity(1200), true)
  await decoder.drain()
  await decoder.dispose()
  const encoder = new RttyEncoder(12000)
  assert.throws(() => encoder.retractUnsent(-1), /RASTERWAVE_INVALID_ARGUMENT/)
  encoder.dispose()
})

test('a throwing listener fails the decoder session', async () => {
  const { pcm } = encodeText('CQ CQ TEST\n')
  const decoder = new RttyDecoder(12000, null, event => {
    if (event.type === 'character') throw new Error('listener bug')
  })
  const onUncaught = () => {}
  process.on('uncaughtException', onUncaught)
  try {
    let offset = 0
    let failed = false
    while (offset < pcm.length && !failed) {
      try {
        if (decoder.pushF32(pcm.subarray(offset, offset + 1200))) offset += 1200
        else await decoder.drain()
      } catch (error) {
        assert.match(String(error), /RASTERWAVE_FAILED/)
        failed = true
      }
    }
    await decoder.drain().catch(() => undefined)
    assert.throws(() => decoder.pushF32(new Float32Array(10)), /RASTERWAVE_FAILED/)
  } finally {
    process.off('uncaughtException', onUncaught)
    await decoder.dispose().catch(() => undefined)
  }
})

const characters = events => events.filter(e => e.type === 'character').map(e => [e.ch, e.startSample])

/** Pushes `steps` (Float32Array chunks or functions called with the decoder) without awaiting in between. */
async function runQueued(steps, options = { queueCapacitySamples: 240000 }) {
  const events = []
  const decoder = new RttyDecoder(12000, options, event => events.push(event))
  for (const step of steps) {
    if (typeof step === 'function') step(decoder)
    // Small chunks keep several pushes queued behind the one being processed.
    else for (let i = 0; i < step.length; i += 1200) assert.equal(decoder.pushF32(step.subarray(i, i + 1200)), true)
  }
  await decoder.finish()
  await decoder.dispose()
  return events
}

test('markDiscontinuity is ordered with queued audio while the worker is busy', async () => {
  const { pcm } = encodeText('RYRYRY CQ CQ DE BG5DRB K')
  const half = 30000
  const before = pcm.subarray(0, half)
  const after = pcm.subarray(half)
  // Reference: the first half is fully processed before the gap is declared.
  const events = []
  const reference = new RttyDecoder(12000, { queueCapacitySamples: 240000 }, event => events.push(event))
  reference.pushF32(before)
  await reference.drain()
  reference.markDiscontinuity(12000)
  reference.pushF32(after)
  await reference.finish()
  await reference.dispose()
  const expected = characters(events)
  // Busy: both halves and the gap are queued at once; the gap applies between them.
  const busy = characters(await runQueued([before, d => assert.equal(d.markDiscontinuity(12000), true), after]))
  assert.deepEqual(busy, expected)
  assert.ok(busy.some(([, start]) => start < half), 'characters before the gap')
  const later = busy.filter(([, start]) => start >= half)
  assert.ok(later.length > 0, 'characters after the gap')
  assert.ok(later.every(([, start]) => start >= half + 12000), 'audio after the gap is shifted by the gap')
  // Without the gap the same characters sit 12000 samples earlier (within re-acquisition jitter).
  const plain = characters(await runQueued([before, after])).slice(-4)
  const shifted = busy.slice(-4)
  assert.deepEqual(shifted.map(([ch]) => ch), plain.map(([ch]) => ch))
  shifted.forEach(([, start], i) => assert.ok(Math.abs(start - 12000 - plain[i][1]) <= 60, `${start} vs ${plain[i][1]}`))
})

test('markDiscontinuity is kept while the queue is full and only reset() before it discards it', async () => {
  const { pcm } = encodeText('CQ CQ DE BG5DRB K')
  const silence = new Float32Array(2400)
  const reference = characters(await runQueued([silence, pcm]))
  assert.ok(reference.length > 0)
  // Full queue: further audio is refused, the gap is still accepted and applied in order.
  const events = []
  const decoder = new RttyDecoder(12000, { queueCapacitySamples: 2400 }, event => events.push(event))
  assert.equal(decoder.pushF32(silence), true)
  assert.equal(decoder.pushF32(new Float32Array(1)), false)
  assert.equal(decoder.markDiscontinuity(5000), true)
  assert.equal(decoder.markDiscontinuity(1000), true)
  await pushAll(decoder, pcm)
  await decoder.finish()
  await decoder.dispose()
  assert.deepEqual(characters(events), reference.map(([ch, start]) => [ch, start + 6000]))
  // A gap declared before reset() is discarded with it; one declared after it is kept.
  const cleared = characters(await runQueued([silence, d => d.markDiscontinuity(7000), d => d.reset(), silence, pcm]))
  assert.deepEqual(cleared, reference)
  const kept = characters(await runQueued([silence, d => d.reset(), d => d.markDiscontinuity(7000), silence, pcm]))
  assert.deepEqual(kept, reference.map(([ch, start]) => [ch, start + 7000]))
})

test('retune and updateSettings are validated in full before queueing', async () => {
  const events = []
  const decoder = new RttyDecoder(12000, null, event => events.push(event))
  assert.throws(() => decoder.retune(1000, 2500), /RASTERWAVE_INVALID_ARGUMENT.*shift/)
  assert.throws(() => decoder.retune(1000, 1010), /RASTERWAVE_INVALID_ARGUMENT/)
  assert.throws(() => decoder.retune(NaN, 1170), /RASTERWAVE_INVALID_ARGUMENT/)
  assert.throws(() => decoder.retune(5900, 6100), /RASTERWAVE_INVALID_ARGUMENT/)
  assert.throws(() => decoder.updateSettings({ lanes: [] }), /RASTERWAVE_INVALID_ARGUMENT/)
  assert.throws(() => decoder.updateSettings({ characterSet: 'ebcdic' }), /RASTERWAVE_INVALID_ARGUMENT/)
  // The session is untouched and a valid shift change works.
  assert.equal(decoder.retune(1000, 2200), true)
  assert.equal(decoder.retune(1387.5, 1812.5), true)
  const { pcm } = encodeText('RYRY WIDE SHIFT\n', 12000, { shiftHz: 425, markHz: 1387.5, spaceHz: 1812.5 })
  await pushAll(decoder, pcm)
  await pushAll(decoder, new Float32Array(6000))
  await decoder.finish()
  await decoder.dispose()
  assert.equal(events.filter(e => e.type === 'error').length, 0)
  assert.match(textOf(events), /WIDE SHIFT/)
})

test('repeated dispose() calls all resolve', async () => {
  const decoder = new RttyDecoder(12000, null, () => {})
  for (let i = 0; i < 4; i += 1) decoder.pushF32(new Float32Array(12000))
  const pending = decoder.drain().then(() => 'resolved', error => String(error))
  const outcomes = await Promise.allSettled([decoder.dispose(), decoder.dispose(), decoder.dispose()])
  assert.deepEqual(outcomes.map(o => o.status), ['fulfilled', 'fulfilled', 'fulfilled'])
  assert.match(await pending, /resolved|RASTERWAVE_DISPOSED/)
  await decoder.dispose()

  const scanner = new RttyScanner(12000, null, () => {})
  for (let i = 0; i < 4; i += 1) scanner.pushF32(new Float32Array(12000))
  const scanned = await Promise.allSettled([scanner.dispose(), scanner.dispose()])
  assert.deepEqual(scanned.map(o => o.status), ['fulfilled', 'fulfilled'])
  await scanner.dispose()
})

test('signal reports carry a calibrated SNR', async () => {
  const { pcm } = encodeText('RYRYRYRY CQ CQ DE BG5DRB BG5DRB PSE K '.repeat(3))
  const reports = []
  for (const snr of [0, 20]) {
    // White noise for the given SNR in 3 kHz; the encoder's default amplitude is constant.
    const power = pcm.reduce((sum, x) => sum + x * x, 0) / pcm.length
    const sigma = Math.sqrt(power / 10 ** (snr / 10) * 6000 / 3000)
    let seed = 99
    const gaussian = () => {
      seed = (seed * 1103515245 + 12345) >>> 0
      const u1 = Math.max(seed / 4294967296, 1e-12)
      seed = (seed * 1103515245 + 12345) >>> 0
      return Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * seed / 4294967296)
    }
    const noisy = pcm.map(x => x + sigma * gaussian())
    const events = await decode(noisy)
    const signals = events.filter(e => e.type === 'signal')
    const middle = signals.slice(signals.length / 4, (signals.length * 3) / 4)
    const mean = middle.reduce((sum, e) => sum + e.snrDb, 0) / middle.length
    reports.push(mean)
    assert.ok(Math.abs(mean - snr) <= 3, `true ${snr} dB reported as ${mean.toFixed(1)} dB`)
  }
  assert.ok(reports[1] > reports[0] + 15)
})

/** Mixes signals (each starting at its offset) over deterministic low-level noise. */
function mixSignals(signals, length) {
  const out = new Float32Array(length)
  let seed = 12345
  for (let i = 0; i < length; i += 1) {
    seed = (seed * 1103515245 + 12345) >>> 0
    out[i] = (seed / 4294967296 - 0.5) * 0.05
  }
  for (const { pcm, offset } of signals) {
    for (let i = 0; i < pcm.length && offset + i < length; i += 1) out[offset + i] += pcm[i] * 0.5
  }
  return out
}

const scannerText = events => {
  const texts = new Map()
  for (const event of events) {
    if (event.type !== 'text') continue
    texts.set(event.channelId, (texts.get(event.channelId) ?? '') + event.characters.map(c => c.ch).join(''))
  }
  return texts
}

test('scanner finds two encoded signals mixed together and previews their text', async () => {
  const texts = ['CQ CQ DE BG5DRB BG5DRB PSE K THE QUICK BROWN FOX JUMPS ', 'QRZ DE JA1XYZ UR 599 599 NAME TARO HW CPY KN TU 73 ']
  const centers = [1000, 1800]
  const signals = centers.map((center, index) => ({
    pcm: encodeText(texts[index], 12000, { markHz: center - 85, spaceHz: center + 85 }).pcm,
    offset: 6000 + index * 2000,
  }))
  const length = Math.max(...signals.map(s => s.offset + s.pcm.length)) + 12000
  const events = []
  const scanner = new RttyScanner(12000, { maxChannels: 4 }, event => events.push(event))
  await pushAll(scanner, mixSignals(signals, length), 1200)
  await scanner.finish()
  await scanner.dispose()

  const lists = events.filter(e => e.type === 'channels')
  assert.ok(lists.length >= 2 * Math.floor(length / 12000))
  const previews = scannerText(events)
  for (const [index, center] of centers.entries()) {
    const channel = lists.flatMap(e => e.channels).find(c => Math.abs(c.centerHz - center) <= 10)
    assert.ok(channel, `no channel near ${center} Hz`)
    assert.equal(channel.shiftHz, 170)
    assert.equal(typeof channel.score, 'number')
    assert.equal(typeof channel.snrDb, 'number')
    assert.equal(channel.reversed, false)
    assert.ok(lists.some(e => e.channels.some(c => c.id === channel.id && c.locked)))
    const preview = previews.get(channel.id) ?? ''
    assert.match(preview, new RegExp(texts[index].trim().split(' ').slice(-4).join(' ')))
    const characters = events.filter(e => e.type === 'text' && e.channelId === channel.id).flatMap(e => e.characters)
    assert.ok(characters.every(c => typeof c.confidence === 'number' && Number.isInteger(c.startSample)))
    assert.ok(characters.every((c, i) => i === 0 || c.startSample > characters[i - 1].startSample))
    // Positions are on the scanner's input timeline: the last character lies inside its transmission.
    assert.ok(characters.at(-1).startSample > signals[index].offset)
    assert.ok(characters.at(-1).startSample < signals[index].offset + signals[index].pcm.length)
  }
  assert.ok(lists.every(e => e.channels.length <= 2))
  assert.equal(events.at(-1).type, 'drain')
})

test('scanner applies backpressure without dropping audio', async () => {
  const events = []
  const scanner = new RttyScanner(12000, { queueCapacitySamples: 2400 }, event => events.push(event))
  assert.throws(() => scanner.pushF32(new Float32Array(2401)), /RASTERWAVE_CHUNK_TOO_LARGE/)
  const chunk = new Float32Array(1200)
  let refused = false
  let accepted = 0
  for (let i = 0; i < 200 && !refused; i += 1) {
    if (scanner.pushF32(chunk)) accepted += chunk.length
    else refused = true
    assert.ok(scanner.queuedSamples <= 2400)
  }
  assert.ok(refused, 'a full queue must refuse audio')
  await scanner.drain()
  assert.equal(scanner.queuedSamples, 0)
  assert.equal(scanner.pushF32(chunk), true)
  accepted += chunk.length
  await scanner.finish()
  const lists = events.filter(e => e.type === 'channels')
  // One list per ~0.5 s of accepted audio plus the final one from finish().
  assert.equal(lists.length, Math.floor(accepted / 5888) + 1)
  assert.equal(lists.at(-1).sample, accepted)
  assert.ok(lists.every(e => e.channels.length === 0))
  assert.ok(events.some(e => e.type === 'finished'))
  assert.throws(() => scanner.pushF32(chunk), /RASTERWAVE_FINISHED/)
  await scanner.dispose()
})

test('scanner dispose cancels queued work and rejects further input', async () => {
  const scanner = new RttyScanner(12000, null, () => {})
  assert.equal(scanner.pushF32(new Float32Array(12000)), true)
  assert.equal(scanner.reset(), true)
  const pending = scanner.drain().then(() => 'resolved', error => String(error))
  await scanner.dispose()
  const outcome = await pending
  assert.ok(outcome === 'resolved' || /RASTERWAVE_DISPOSED/.test(outcome), outcome)
  assert.throws(() => scanner.pushF32(new Float32Array(10)), /RASTERWAVE_DISPOSED/)
  assert.throws(() => scanner.reset(), /RASTERWAVE_DISPOSED/)
  await scanner.dispose()
})

test('scanner rejects invalid configuration and input', async () => {
  assert.throws(() => new RttyScanner(12000, { shiftsHz: [] }, () => {}), /RASTERWAVE_INVALID_CONFIG/)
  assert.throws(() => new RttyScanner(12000, { maxChannels: 0 }, () => {}), /RASTERWAVE_INVALID_CONFIG/)
  assert.throws(() => new RttyScanner(12000, { minHz: 1000, maxHz: 1100 }, () => {}), /RASTERWAVE_INVALID_CONFIG/)
  assert.throws(() => new RttyScanner(12000, { minScore: 2 }, () => {}), /RASTERWAVE_INVALID_CONFIG/)
  assert.throws(() => new RttyScanner(12000, { stopBits: 3 }, () => {}), /RASTERWAVE_INVALID_CONFIG/)
  assert.throws(() => new RttyScanner(12000, { queueCapacitySamples: 0 }, () => {}), /RASTERWAVE_INVALID_CONFIG/)
  const scanner = new RttyScanner(12000, { shiftsHz: [170, 425], baud: 50 }, () => {})
  assert.throws(() => scanner.pushF32(new Float32Array([0, NaN])), /RASTERWAVE_NON_FINITE_SAMPLE/)
  await scanner.dispose()
})

test('scanner finds a 425 Hz shift signal when the shift is configured', async () => {
  const options = { baud: 50, shiftHz: 425 }
  const { pcm } = encodeText('RYRY WIDE SHIFT TEST DE N0CALL N0CALL K ', 12000, { ...options, markHz: 1387.5, spaceHz: 1812.5 })
  const events = []
  const scanner = new RttyScanner(12000, options, event => events.push(event))
  await pushAll(scanner, mixSignals([{ pcm, offset: 6000 }], pcm.length + 18000), 1200)
  await scanner.finish()
  await scanner.dispose()
  const channel = events.filter(e => e.type === 'channels').flatMap(e => e.channels).find(c => Math.abs(c.centerHz - 1600) <= 10)
  assert.ok(channel)
  assert.equal(channel.shiftHz, 425)
  assert.match(scannerText(events).get(channel.id) ?? '', /DE N0CALL/)
})
