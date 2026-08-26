import {
  FaxDecoder,
  FaxIoc,
  correctFaxPaper,
  SstvDecoder,
  SstvEncoder,
  SstvMode,
  type FaxDecodeEvent,
  type SstvDecodeEvent,
} from '..'

const sstvEvents: SstvDecodeEvent[] = []
const decoder = new SstvDecoder(12000, {
  outputMode: 'continuousPaper',
  fallbackMode: SstvMode.Robot36,
}, event => sstvEvents.push(event))
const accepted: boolean = decoder.pushF32(new Float32Array(10))
void accepted
void decoder.drain()

const encoder = new SstvEncoder(new Uint8Array(160 * 120 * 3), SstvMode.Robot8Bw, 12000, {
  enhancedPreamble: true,
  stationId: { kind: 'cw', callsign: 'BG5DRB', wpm: 20, toneHz: 800 },
  postImageGapMs: 500,
  endGuardMs: 300,
})
const samples: Promise<Float32Array> = encoder.readSamples(4096)
void samples
const rasterEnd: number = encoder.progress.rasterEndSample
void rasterEnd

const faxEvents: FaxDecodeEvent[] = []
new FaxDecoder(12000, {
  outputMode: 'continuousPaper',
  continuousAuto: false,
  ioc: FaxIoc.Ioc288,
  lpm: 120,
}, event => faxEvents.push(event))

const corrected: Promise<Uint8Array> = correctFaxPaper(
  new Uint8Array(8), 4, 2, 0,
  [{ revision: 1, referenceLine: 0, phasePixels: 0, clockPpm: 0, confidence: 1, source: 'phasing', status: 'locked' }],
)
void corrected
