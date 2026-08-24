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

## Radiofax

`FaxDecoder` uses the same synchronous write and event callback model.
`FaxEncoder` accepts packed grayscale pixels plus IOC, LPM, modulation, and
framing parameters. Both FM and AM subcarriers are supported.

```js
const { FaxEncoder, FaxIoc } = require('rasterwave-node')

const encoder = new FaxEncoder(
  grayscale,
  864,
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
