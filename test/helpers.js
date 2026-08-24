'use strict'

const { SstvEncoder, FaxEncoder } = require('..')

async function readAll(encoder, chunkSize = 4096) {
  const chunks = []
  while (!encoder.isFinished) chunks.push(await encoder.readSamples(chunkSize))
  const output = new Float32Array(chunks.reduce((total, chunk) => total + chunk.length, 0))
  let offset = 0
  for (const chunk of chunks) {
    output.set(chunk, offset)
    offset += chunk.length
  }
  await encoder.dispose()
  return output
}

async function encodeSstv(pixels, mode, sampleRate = 12000) {
  return readAll(new SstvEncoder(pixels, mode, sampleRate, null))
}

async function encodeFax(pixels, width, height, spec, options = null, sampleRate = 12000) {
  return readAll(new FaxEncoder(pixels, width, height, spec, sampleRate, options))
}

async function pushAll(decoder, samples, chunkSize = 731) {
  let offset = 0
  while (offset < samples.length) {
    const chunk = samples.subarray(offset, Math.min(offset + chunkSize, samples.length))
    if (decoder.pushF32(chunk)) offset += chunk.length
    else await decoder.drain()
  }
}

module.exports = { encodeFax, encodeSstv, pushAll, readAll }
