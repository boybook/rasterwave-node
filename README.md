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
