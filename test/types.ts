import {
  FaxDecoder,
  FaxIoc,
  SstvDecoder,
  SstvEncoder,
  SstvMode,
  type FaxDecodeEvent,
  type SstvDecodeEvent,
} from '..'

const sstvEvents: SstvDecodeEvent[] = []
const decoder = new SstvDecoder(12000, {
  immediateDecode: true,
  manualMode: SstvMode.Robot8Bw,
}, event => sstvEvents.push(event))
const accepted: boolean = decoder.pushF32(new Float32Array(10))
void accepted
void decoder.drain()

const encoder = new SstvEncoder(new Uint8Array(160 * 120 * 3), SstvMode.Robot8Bw, 12000)
const samples: Promise<Float32Array> = encoder.readSamples(4096)
void samples

const faxEvents: FaxDecodeEvent[] = []
new FaxDecoder(12000, {
  immediateDecode: true,
  ioc: FaxIoc.Ioc288,
  lpm: 120,
}, event => faxEvents.push(event))
