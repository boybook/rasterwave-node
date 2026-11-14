# rasterwave-node

Native Node.js bindings for the [`rasterwave`](https://github.com/boybook/rasterwave.rs)
streaming SSTV and radiofax codec. CPU-heavy work runs on a bounded Rust thread
pool; no Node.js worker thread is required.

## Install

```bash
npm install rasterwave-node
```

Prebuilt Node-API binaries are published for macOS arm64/x64, Linux glibc
arm64/x64, and Windows x64. Node.js 20.17 or newer is required.

## Streaming SSTV decode

```js
const { SstvDecoder } = require('rasterwave-node')

const decoder = new SstvDecoder(48000, null, event => {
  if (event.type === 'lineReady') {
    // event.pixels is packed RGB data for one displayable row.
    drawRow(event.lineIndex, event.pixels, event.revision)
  }
})

function acceptAudio(samples) {
  if (!decoder.pushF32(samples)) return decoder.drain().then(() => acceptAudio(samples))
}

await decoder.finish()
await decoder.dispose()
```

`pushF32()` only copies and queues PCM, so it returns synchronously. It returns
`false` without consuming the chunk when the bounded queue is full. Decode
events are emitted while input is still arriving; `finish()` is not required
before rows become available.

Set `immediateDecode: true` together with `manualMode` to start a forced SSTV
preview with the first PCM sample. All built-in SSTV modes support this path;
later sync pulses still correct clock, phase, and frequency offset.

Set `outputMode: 'continuousPaper'` for a receiver-style paper raster. Auto
prints Robot 36 Color rows immediately while VIS and sync acquisition run in
parallel. Trusted boundaries open protocol captures, but paper row numbers keep
growing after `transmissionCompleted`.

## Streaming SSTV encode

```js
const { SstvEncoder, SstvMode, sstvModes } = require('rasterwave-node')

const mode = SstvMode.Robot36
const spec = sstvModes().find(item => item.mode === mode)
const rgb = new Uint8Array(spec.width * spec.height * 3)
const encoder = new SstvEncoder(rgb, mode, 48000)

while (!encoder.isFinished) {
  const pcm = await encoder.readSamples(4096)
  await audioOutput.write(pcm)
}
await encoder.dispose()
```

Concurrent `readSamples()` calls are accepted and resolved in call order.

Radio applications can add a QSSTV-compatible calibration preamble and a
post-image station identifier without concatenating JavaScript audio buffers:

```js
const encoder = new SstvEncoder(rgb, mode, 48000, {
  enhancedPreamble: true,
  stationId: { kind: 'fsk', callsign: 'N0CALL' },
  postImageGapMs: 500,
  endGuardMs: 300,
})

const { rasterStartSample, rasterEndSample } = encoder.progress
```

Use `{ kind: 'cw', callsign: 'N0CALL', wpm: 20, toneHz: 800 }` for an audible
Morse ID or `{ kind: 'none' }` for no ID. The legacy constructor remains
sample-identical and still includes the standard VIS header by default.

## Radiofax

`FaxDecoder` uses the same synchronous write and event callback model.
`FaxEncoder` accepts packed grayscale pixels plus IOC, LPM, modulation, and
framing parameters. Both FM and AM subcarriers are supported.

`FaxDecoder` also accepts `immediateDecode: true` with fixed `ioc` and `lpm`.
It begins raster output without waiting for APT/phasing and continues through
weak signal intervals, which supports joining a weatherfax transmission in
progress.

With `outputMode: 'continuousPaper'`, fax starts from IOC576/120 LPM/FM (or the
provided fallback), evaluates IOC/LPM and FM/AM acquisition in parallel, and
continues printing after confirmed APT stop. `markSignalLost()` inserts a
discontinuity boundary instead of completing a page in this mode.

Radiofax clock recovery is enabled by default. Continuous-paper rows use the
stable `nominalPaper` basis until trusted phasing is acquired; phasing captures
then emit `calibrated` rows directly and are never retimed by ordinary image
content. A mid-image join may report sparse `imageContent` points only after
stable evidence; this is a heuristic, not a protocol lock. `correctFaxPaper()`
applies points on the native Rayon pool and returns a Promise, so correcting a
long nominal capture does not block the Node event loop. Do not apply it again
to `calibrated` rows.

```js
const { FaxEncoder, FaxIoc } = require('rasterwave-node')

const encoder = new FaxEncoder(
  grayscale,
  905,
  pageHeight,
  { ioc: FaxIoc.Ioc288, lpm: 120 },
  12000,
)
```

See [`index.d.ts`](./index.d.ts) for the complete event and configuration API.

## RTTY

```js
const { RttyEncoder, RttyDecoder } = require('rasterwave-node')

// Synchronous: calls run in program order, so retraction never races a read.
const encoder = new RttyEncoder(12000, { markHz: 2125, spaceHz: 2295 })
encoder.pushText('CQ CQ DE N0CALL K')
const { samples, rendered } = encoder.readSamples(1200) // rendered: committed characters
encoder.retractUnsent(1) // only characters not yet committed
encoder.finish()

const decoder = new RttyDecoder(12000, { squelch: 0.25 }, event => {
  if (event.type === 'character') process.stdout.write(event.ch)
})
decoder.pushF32(samples) // false means the queue is full: await decoder.drain()
```

The encoder sends LTRS diddles while its buffer is empty and only ends after
`finish()` or `abort()` (`abort()` also cuts a tail already started by
`finish()`). Tones are audio frequencies; choose mark above or below space
according to the radio sideband.

Decoder control calls (`retune`, `setSquelch`, `updateSettings`, `reset`,
`markDiscontinuity`) are validated synchronously, throwing
`RASTERWAVE_INVALID_ARGUMENT`, and then queued in order with the audio.
`markDiscontinuity(n)` declares `n` lost samples between the audio pushed
before and after the call; it is always accepted, even while the queue is full.

`signal` events report `snrDb`, the SNR in a 3 kHz bandwidth measured at the bit
centres of decoded characters (within about 1 dB of the true value from −12 to
+40 dB on white noise; around −13 to −16 dB for noise alone), and `markDb` /
`spaceDb`, fast per-tone levels for tuning and tone balance that saturate near
15–16 dB. Digital silence (for example a receiver muted during transmit) freezes
the decoder's level estimates, so noise after the silence is not mistaken for a
signal.

### RTTY signal browser

`RttyScanner` searches the whole audio passband for RTTY signals and decodes a
short preview of each, so a user can pick one and tune the main decoder there.
It uses the same queue, backpressure and `drain`/`finish`/`dispose` contract as
the decoders.

```js
const { RttyScanner } = require('rasterwave-node')

const scanner = new RttyScanner(12000, { shiftsHz: [170, 425], maxChannels: 8 }, event => {
  switch (event.type) {
    case 'channels': // every ~500 ms, ordered by centre, also when empty
      for (const c of event.channels) console.log(c.id, c.centerHz, c.shiftHz, c.score, c.snrDb, c.locked, c.reversed)
      break
    case 'text': // preview characters for one channel
      console.log(event.channelId, event.characters.map(c => c.ch).join(''))
      break
    case 'removed': // the id will not appear again
      console.log('gone', event.channelId)
      break
  }
})
scanner.pushF32(samples) // false: await scanner.drain() and retry
```

Detection compares the energy of tone pairs one shift apart over the last
1.5 s: RTTY moves its energy between mark and space, which separates it from
two carriers or voice. A candidate is reported after about a second of stable
detection and dropped three seconds after it disappears; ids are never reused.
`reversed: false` means mark is the lower audio tone, so the main decoder can be
tuned with `markHz = centerHz - shiftHz / 2` and `spaceHz = centerHz + shiftHz / 2`
(swap them when `reversed` is true). Options: `minHz`/`maxHz` bound both tones
(default 300 to 2900 Hz), `minScore` (default 0.5), and `baud`/`shiftHz`/
`stopBits` configure the preview decoders (`shiftsHz` defaults to `[shiftHz]`).

## Concurrency and memory

- A process-wide Rust pool uses up to four threads.
- Each codec instance is a FIFO actor; its mutable codec state is never accessed concurrently.
- Decoder input defaults to five seconds of PCM and can be changed with `queueCapacitySamples`.
- JavaScript typed arrays are copied before asynchronous work begins.
- Event callbacks are bounded and apply native backpressure rather than dropping rows.

## Development

```bash
npm install
npm run build:debug
npm test
npm run test:types
cargo test
```

Rust 1.88 is pinned in `rust-toolchain.toml`.

## License

MIT
