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

import { RttyDecoder, RttyEncoder, rttyProfiles, type RttyDecodeEvent, type RttySignalReport } from '..'
const rttyEvents: RttyDecodeEvent[] = []
const rttyDecoder = new RttyDecoder(12000, { squelch: 0.25, lanes: ['atc'], markHz: 2125, spaceHz: 2295 }, event => rttyEvents.push(event))
const rttyAccepted: boolean = rttyDecoder.pushF32(new Float32Array(10))
void rttyAccepted
const rttyEncoder = new RttyEncoder(12000, { idle: 'diddle', stopBits: 1.5 })
const firstSeq: number | undefined = rttyEncoder.pushText('CQ').firstSeq
void firstSeq
const read = rttyEncoder.readSamples(1200)
const committed: string[] = read.rendered.map(item => item.ch)
void committed
const profileId: string = rttyProfiles()[0].id
void profileId
const rttySignal = rttyEvents.find((event): event is RttySignalReport => event.type === 'signal')
const snr: number | undefined = rttySignal?.snrDb
void snr
const rttyEncoderLtrs = new RttyEncoder(12000, { preambleLtrs: 2, tailLtrs: 1 })
void rttyEncoderLtrs
const disposed: Promise<void> = rttyDecoder.dispose()
void disposed

import { RttyScanner, type RttyScanEvent, type RttyScanChannel } from '..'
const scanEvents: RttyScanEvent[] = []
const scanner = new RttyScanner(12000, { shiftsHz: [170, 425], minHz: 300, maxHz: 2900, maxChannels: 8, minScore: 0.5, baud: 45.45, stopBits: 1.5, queueCapacitySamples: 60000 }, event => {
  scanEvents.push(event)
  switch (event.type) {
    case 'channels': {
      const list: RttyScanChannel[] = event.channels
      const center: number = list[0].centerHz
      const flags: boolean = list[0].reversed && list[0].locked
      void center; void flags
      break
    }
    case 'text': {
      const preview: string = event.characters.map(c => c.ch).join('')
      const id: number = event.channelId
      const start: number = event.characters[0].startSample
      void preview; void id; void start
      break
    }
    case 'removed': {
      const removedId: number = event.channelId
      void removedId
      break
    }
    case 'drain':
    case 'finished':
      break
    case 'error': {
      const reason: string = event.reason
      void reason
      break
    }
    default: {
      const exhaustive: never = event
      void exhaustive
    }
  }
})
const scanAccepted: boolean = scanner.pushF32(new Float32Array(10))
void scanAccepted
const scanQueued: number = scanner.queuedSamples
void scanQueued
void scanner.reset()
void scanner.drain()
void scanner.finish()
void scanner.dispose()
