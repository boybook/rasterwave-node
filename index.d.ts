export const SstvMode: {
  readonly Robot8Bw: 'robot8Bw'; readonly Robot12Bw: 'robot12Bw'; readonly Robot24Bw: 'robot24Bw'; readonly Robot36Bw: 'robot36Bw'
  readonly Robot12: 'robot12'; readonly Robot24: 'robot24'; readonly Robot36: 'robot36'; readonly Robot72: 'robot72'
  readonly Martin1: 'martin1'; readonly Martin2: 'martin2'; readonly Martin3: 'martin3'; readonly Martin4: 'martin4'
  readonly Scottie1: 'scottie1'; readonly Scottie2: 'scottie2'; readonly Scottie3: 'scottie3'; readonly Scottie4: 'scottie4'; readonly ScottieDx: 'scottieDx'
  readonly Pd50: 'pd50'; readonly Pd90: 'pd90'; readonly Pd120: 'pd120'; readonly Pd160: 'pd160'; readonly Pd180: 'pd180'; readonly Pd240: 'pd240'; readonly Pd290: 'pd290'
  readonly WraaseSc2_30: 'wraaseSc2_30'; readonly WraaseSc2_60: 'wraaseSc2_60'; readonly WraaseSc2_120: 'wraaseSc2_120'; readonly WraaseSc2_180: 'wraaseSc2_180'
  readonly Pasokon3: 'pasokon3'; readonly Pasokon5: 'pasokon5'; readonly Pasokon7: 'pasokon7'
}
export type SstvMode = (typeof SstvMode)[keyof typeof SstvMode]
export const FaxIoc: { readonly Ioc288: 'ioc288'; readonly Ioc576: 'ioc576' }
export type FaxIoc = (typeof FaxIoc)[keyof typeof FaxIoc]
export const FaxPolarity: { readonly Normal: 'normal'; readonly Inverted: 'inverted' }
export type FaxPolarity = (typeof FaxPolarity)[keyof typeof FaxPolarity]

export interface SstvModeInfo {
  mode: SstvMode; name: string; visCode: number; width: number; height: number
  colorLayout: 'monochrome' | 'rgb' | 'yuv'
  scanLayout: 'monochrome' | 'martin' | 'scottie' | 'robot' | 'pd' | 'wraase' | 'pasokon'
  lineSeconds: number; rowsPerLine: number; status: 'canonical' | 'compatibility'
}
export type DecoderOutputMode = 'framed' | 'continuousPaper'
export type PaperBoundaryKind = 'initial' | 'vis' | 'syncTiming' | 'aptPhasing' | 'protocolEnd' | 'discontinuity' | 'reset' | 'unknown'
export interface SstvDecoderOptions {
  outputMode?: DecoderOutputMode
  fallbackMode?: SstvMode
  immediateDecode?: boolean
  detectVis?: boolean; detectSyncTiming?: boolean; manualMode?: SstvMode
  minimumSignalLevel?: number; queueCapacitySamples?: number
}
export type SstvStationIdOptions =
  | { kind: 'none' }
  | { kind: 'fsk'; callsign: string }
  | { kind: 'cw'; callsign: string; wpm?: number; toneHz?: number }
export interface SstvEncoderOptions {
  amplitude?: number; toneOffsetHz?: number; includeVisHeader?: boolean
  enhancedPreamble?: boolean; stationId?: SstvStationIdOptions
  postImageGapMs?: number; endGuardMs?: number
}

export type SstvDecodeEvent =
  | { type: 'paperStarted'; paperId: number; mode: SstvMode; width: number }
  | { type: 'rasterBoundary'; paperId: number; boundaryId: number; lineIndex: number; mode: SstvMode; detection?: 'vis' | 'syncTiming' | 'manual' | 'unknown'; visCode?: number; ambiguous?: boolean; candidateCount?: number; boundaryKind: PaperBoundaryKind; trusted: boolean; width: number; nominalHeight: number }
  | { type: 'rasterLineReady'; paperId: number; boundaryId: number; lineIndex: number; modeLineIndex: number; mode: SstvMode; revision: number; completeness: 'provisional' | 'final'; pixels: Uint8Array }
  | { type: 'transmissionCompleted'; paperId: number; boundaryId: number; startLine: number; endLine: number; mode: SstvMode; lines: number }
  | { type: 'protocolObserved'; mode: SstvMode; detection: 'vis' | 'syncTiming' | 'manual' | 'unknown'; visCode?: number; ambiguous?: boolean; candidateCount?: number; trusted: boolean }
  | { type: 'modeCandidate'; candidates: SstvMode[]; confidence: number }
  | { type: 'imageStarted'; imageId: number; mode: SstvMode; detection: 'vis' | 'syncTiming' | 'manual' | 'unknown'; visCode?: number; ambiguous?: boolean; candidateCount?: number; frequencyOffsetHz: number; width: number; height: number }
  | { type: 'lineReady'; imageId: number; mode: SstvMode; lineIndex: number; revision: number; completeness: 'provisional' | 'final'; pixels: Uint8Array }
  | { type: 'imageCompleted'; imageId: number; mode: SstvMode; lines: number }
  | { type: 'imageAborted'; imageId: number; mode: SstvMode; lastLine?: number; reason: 'inputDiscontinuity' | 'endOfInput' | 'reset' | 'syncLost' | 'unknown' }
  | { type: 'signalRejected'; reason: string }
  | DecoderControlEvent | DecoderErrorEvent
export type DecoderControlEvent = { type: 'drain' | 'finished' }
export type DecoderErrorEvent = { type: 'error'; reason: string }
export type SstvEncoderStage = 'preamble' | 'vis' | 'raster' | 'stationId' | 'guard' | 'finished'
export interface EncoderProgress {
  samplesEmitted: number; estimatedTotalSamples: number; currentRow?: number
  stage: SstvEncoderStage; rasterStartSample: number; rasterEndSample: number; finished: boolean
}

export class SstvDecoder {
  constructor(inputSampleRate: number, options: SstvDecoderOptions | null | undefined, onEvent: (event: SstvDecodeEvent) => void)
  pushF32(input: Float32Array): boolean
  reset(): boolean
  markDiscontinuity(droppedInputSamples: number): boolean
  drain(): Promise<void>
  finish(): Promise<void>
  dispose(): Promise<void>
  readonly queuedSamples: number
  readonly syncState: 'searching' | 'readingVis' | 'confirming' | 'locked' | 'finished'
}
export class SstvEncoder {
  constructor(pixels: Uint8Array, mode: SstvMode, sampleRate: number, options?: SstvEncoderOptions | null)
  readSamples(maxSamples: number): Promise<Float32Array>
  dispose(): Promise<void>
  readonly isFinished: boolean
  readonly progress: EncoderProgress
}

export interface FaxModulationOptions {
  kind: 'fm' | 'fmSubcarrier' | 'am' | 'amSubcarrier'
  centerHz?: number; deviationHz?: number; polarity?: FaxPolarity
  carrierHz?: number; blackLevel?: number; whiteLevel?: number
}
export interface FaxSpecOptions {
  ioc: FaxIoc; lpm: number; modulation?: FaxModulationOptions
  phasingSeconds?: number; startSeconds?: number; stopSeconds?: number
  trailingBlackSeconds?: number
}
export interface FaxEncoderOptions { amplitude?: number; includeApt?: boolean; includePhasing?: boolean }
export interface FaxDecoderOptions {
  outputMode?: DecoderOutputMode
  clockRecovery?: 'auto' | 'off'
  continuousAuto?: boolean
  autoAmModulation?: FaxModulationOptions
  immediateDecode?: boolean
  ioc?: FaxIoc; lpm?: number; modulation?: FaxModulationOptions; maxLines?: number
  amFullScale?: number; expectedPhasingSeconds?: number; aptConfirmSeconds?: number
  acquisitionTimeoutSeconds?: number; stopConfirmSeconds?: number; signalLossSeconds?: number
  minimumSignalLevel?: number; minimumCarrierCoherence?: number; queueCapacitySamples?: number
}
export type FaxClockSource = 'nominal' | 'phasing' | 'imageContent' | 'manual'
export type FaxClockStatus = 'nominal' | 'acquiring' | 'locked' | 'tracking' | 'degraded'
export type FaxRasterBasis = 'calibrated' | 'nominalPaper'
export interface FaxClockCalibration {
  revision: number; referenceLine: number; phasePixels: number; clockPpm: number
  confidence: number; source: FaxClockSource; status: FaxClockStatus
}
export interface FaxPaperCorrection { phasePixels?: number; clockPpm?: number }
export type FaxDecodeEvent =
  | { type: 'paperStarted'; paperId: number; ioc: FaxIoc; lpm: number; width: number; activeWidth: number; modulation: 'fm' | 'am' }
  | { type: 'rasterBoundary'; paperId: number; boundaryId: number; lineIndex: number; ioc: FaxIoc; lpm: number; width: number; activeWidth: number; modulation: 'fm' | 'am'; boundaryKind: PaperBoundaryKind; trusted: boolean }
  | ({ type: 'clockCalibration'; paperId: number; boundaryId: number } & FaxClockCalibration)
  | { type: 'rasterLineReady'; paperId: number; boundaryId: number; lineIndex: number; segmentLineIndex: number; ioc: FaxIoc; lpm: number; width: number; activeWidth: number; modulation: 'fm' | 'am'; basis: FaxRasterBasis; pixels: Uint8Array }
  | { type: 'transmissionCompleted'; paperId: number; boundaryId: number; startLine: number; endLine: number; ioc: FaxIoc; lpm: number; width: number; activeWidth: number; modulation: 'fm' | 'am'; lines: number; partial: false }
  | { type: 'protocolObserved'; ioc: FaxIoc; lpm: number; width: number; activeWidth: number; modulation: 'fm' | 'am'; trusted: boolean }
  | { type: 'aptDetected'; ioc: FaxIoc }
  | ({ type: 'phasingLocked'; ioc: FaxIoc; lpm: number; width: number } & FaxClockCalibration)
  | ({ type: 'pageStarted'; pageId: number; ioc: FaxIoc; lpm: number; width: number; activeWidth: number; modulation: 'fm' | 'am' } & FaxClockCalibration)
  | { type: 'lineReady'; pageId: number; lineIndex: number; basis: FaxRasterBasis; pixels: Uint8Array }
  | { type: 'pageCompleted'; pageId: number; lines: number; partial: boolean }
  | { type: 'signalRejected'; reason: string }
  | DecoderControlEvent | DecoderErrorEvent
export interface FaxEncoderProgress { samplesEmitted: number; finished: boolean }
export class FaxDecoder {
  constructor(inputSampleRate: number, options: FaxDecoderOptions | null | undefined, onEvent: (event: FaxDecodeEvent) => void)
  pushF32(input: Float32Array): boolean
  reset(): boolean
  markSignalLost(): boolean
  drain(): Promise<void>
  finish(): Promise<void>
  dispose(): Promise<void>
  readonly queuedSamples: number
  readonly clockState: FaxClockCalibration
}
export function correctFaxPaper(pixels: Uint8Array, width: number, height: number, startLine: number, calibration: FaxClockCalibration[], adjustment?: FaxPaperCorrection | null): Promise<Uint8Array>
export class FaxEncoder {
  constructor(pixels: Uint8Array, width: number, height: number, spec: FaxSpecOptions, sampleRate: number, options?: FaxEncoderOptions | null)
  readSamples(maxSamples: number): Promise<Float32Array>
  dispose(): Promise<void>
  readonly isFinished: boolean
  readonly progress: FaxEncoderProgress
}
export function sstvModes(): SstvModeInfo[]

export type RttyCharacterSet = 'usTty' | 'ita2'
export type RttyStopBits = 1 | 1.5 | 2
export type RttyLane = 'atc' | 'kahn' | 'markOnly' | 'spaceOnly'
/** Reported lanes may include values added by a newer native core. */
export type RttyReportedLane = RttyLane | 'unknown'
export interface RttyProfileInfo { id: string; baud: number; shiftHz: number; stopBits: RttyStopBits }
export interface RttySignalOptions {
  baud?: number; shiftHz?: number; stopBits?: RttyStopBits
  /** Audio mark tone; give together with spaceHz. Default: 2210 Hz minus half the shift. */
  markHz?: number
  /** Audio space tone; give together with markHz. Default: 2210 Hz plus half the shift. */
  spaceHz?: number
  characterSet?: RttyCharacterSet
}
export interface RttyEncoderOptions extends RttySignalOptions {
  amplitude?: number; preambleMarkMs?: number
  /** LTRS characters sent before the text, integer 0..=32. */
  preambleLtrs?: number
  idle?: 'diddle' | 'mark'
  /** LTRS characters sent after the text on finish(), integer 0..=32. */
  tailLtrs?: number
  tailMarkMs?: number
  transitionMs?: number; envelopeMs?: number; expandNewline?: boolean
}
export interface RttyDecoderOptions extends RttySignalOptions {
  usos?: boolean; afc?: boolean; afcRangeHz?: number
  /** 0 disables; characters with lower confidence are suppressed. Default 0.25. */
  squelch?: number
  /** Decision rules run side by side; default ['atc', 'kahn']. */
  lanes?: RttyLane[]; autoReverse?: boolean; signalReportMs?: number
  /** Remove short impulses (static crashes). Default true. */
  noiseBlanker?: boolean
  queueCapacitySamples?: number
}
export interface RttyDecoderSettings {
  afc?: boolean; usos?: boolean; characterSet?: RttyCharacterSet
  lanes?: RttyLane[]; autoReverse?: boolean
}
/** `rejected` holds characters without an ITA2 code or beyond the 4096-character buffer. */
export interface RttyPushResult { accepted: number; firstSeq?: number; rejected: string }
export interface RttyRenderedCharacter { seq: number; ch: string; startSample: number; endSample: number }
export interface RttyReadResult { samples: Float32Array; rendered: RttyRenderedCharacter[]; finished: boolean }
export interface RttySignalReport {
  type: 'signal'
  /** Input sample at which the report was produced. */
  sample: number
  /** True while recent characters are decoded with good confidence; AFC only adapts while locked. */
  locked: boolean
  /**
   * Signal-to-noise ratio in a 3 kHz noise bandwidth, dB. Measured in the power domain at the bit
   * centres of recently decoded characters (both tones, noise scaled from the matched filters'
   * noise bandwidth). Tracks the true SNR within about 1 dB from -12 to +40 dB on white noise;
   * flattens between -13 and -16 dB below that, which is also what noise alone reads. Holds its
   * value while nothing is decoded (silence, steady carrier); -30 before the first measurement.
   */
  snrDb: number
  /**
   * Mark filter level: smoothed amplitude while mark is keyed over the same filter's amplitude while
   * space is keyed, 20·log10((on − off) / off) in the matched-filter bandwidth; -30 at the floor, 0
   * before any level is known. A fast relative tuning/tone-balance indicator, not a calibrated SNR:
   * leakage between adjacent bits caps it near 15–16 dB.
   */
  markDb: number
  /** Space filter level, defined like markDb. */
  spaceDb: number
  /** Correction currently applied by AFC, Hz. */
  frequencyOffsetHz: number
  /** True when recent characters were decoded reversed. */
  reversed: boolean
}
/**
 * `error` ends the session (later calls throw RASTERWAVE_FAILED), except an error whose reason
 * starts with RASTERWAVE_INVALID_ARGUMENT: a queued control command the codec could not apply,
 * which leaves the previous settings in place. Control commands are validated before queueing,
 * so that is not expected to happen.
 */
export type RttyDecodeEvent =
  | { type: 'character'; ch: string; code: number; confidence: number; startSample: number; lane: RttyReportedLane; reversed: boolean }
  | RttySignalReport
  | DecoderControlEvent | DecoderErrorEvent
/** Synchronous interactive encoder; calls run in program order on the caller's thread. */
export class RttyEncoder {
  constructor(sampleRate: number, options?: RttyEncoderOptions | null)
  pushText(text: string): RttyPushResult
  retractUnsent(count: number): number
  clearUnsent(): number
  finish(): void
  abort(): void
  readSamples(maxSamples: number): RttyReadResult
  dispose(): void
  readonly pendingText: string
  readonly pendingLength: number
  readonly isFinished: boolean
  readonly isFinishing: boolean
  readonly samplesRendered: number
  readonly sampleRate: number
}
export class RttyDecoder {
  constructor(inputSampleRate: number, options: RttyDecoderOptions | null | undefined, onEvent: (event: RttyDecodeEvent) => void)
  pushF32(input: Float32Array): boolean
  /**
   * Moves to new tones in order with queued audio. A different shift also becomes the profile shift
   * (20..=1200 Hz). Validated in full before queueing: throws RASTERWAVE_INVALID_ARGUMENT for tones
   * outside the passband, a shift outside 20..=1200 Hz or non-finite values. Returns false (queueing
   * nothing) when the queue is full.
   */
  retune(markHz: number, spaceHz: number): boolean
  /** Throws RASTERWAVE_INVALID_ARGUMENT unless 0..=1. */
  setSquelch(squelch: number): boolean
  /** Validated before queueing; throws RASTERWAVE_INVALID_ARGUMENT for unknown or empty lanes or an unknown character set. */
  updateSettings(settings: RttyDecoderSettings): boolean
  /** Returns to the initial state, in order with queued audio; discontinuities declared earlier are discarded with it. */
  reset(): boolean
  /**
   * Declares that `droppedInputSamples` samples were lost between the audio queued before this call
   * and the audio queued after it (in order with reset() and the other commands). Always accepted,
   * also while the queue is full; character positions stay on the original input timeline.
   */
  markDiscontinuity(droppedInputSamples: number): boolean
  drain(): Promise<void>
  finish(): Promise<void>
  /** Cancels queued work (its promises reject with RASTERWAVE_DISPOSED); every dispose() call resolves once the codec is released. */
  dispose(): Promise<void>
  readonly queuedSamples: number
}
export function rttyProfiles(): RttyProfileInfo[]

export interface RttyScannerOptions {
  /** Shifts to search for, in Hz. Default: [shiftHz], i.e. [170]. At most 8. */
  shiftsHz?: number[]
  /** Lowest audio frequency either tone may occupy. Default 300 Hz. */
  minHz?: number
  /** Highest audio frequency either tone may occupy. Default 2900 Hz. */
  maxHz?: number
  /** Channels reported and decoded at once, 1..=32. Default 8. */
  maxChannels?: number
  /** Detection score (0..1) a candidate needs before it is reported. Default 0.5. */
  minScore?: number
  /** Baud of the per-channel preview decoders. Default 45.45. */
  baud?: number
  /** Profile shift; also the default search shift when shiftsHz is omitted. */
  shiftHz?: number
  stopBits?: RttyStopBits
  queueCapacitySamples?: number
}
export interface RttyScanChannel {
  /** Stable id; never reused by the same scanner, also not after reset(). */
  id: number
  /** Smoothed centre between the tones, Hz. */
  centerHz: number
  shiftHz: number
  /** Smoothed detection score, 0..1. */
  score: number
  /** Spectral SNR estimate in a 3 kHz bandwidth, dB, for ranking channels: reads 1–2 dB low up to about 20 dB and flattens near 25 dB. */
  snrDb: number
  /** True when the preview decoder locked with mark on the upper audio tone; false means mark is the lower tone. */
  reversed: boolean
  /** True while the preview decoder frames characters with good confidence. */
  locked: boolean
}
export interface RttyScanCharacter { ch: string; confidence: number; startSample: number }
/** Discriminated on `type`, like the decoder events. `error` ends the session. */
export type RttyScanEvent =
  /** Current channel list ordered by centre, every ~500 ms (also when empty). */
  | { type: 'channels'; sample: number; channels: RttyScanChannel[] }
  /** Preview characters decoded on one channel since its previous text event. */
  | { type: 'text'; sample: number; channelId: number; characters: RttyScanCharacter[] }
  /** The channel was dropped; its id will not appear again. */
  | { type: 'removed'; sample: number; channelId: number }
  | DecoderControlEvent | DecoderErrorEvent
/** Wideband RTTY signal browser: finds RTTY channels in the passband and decodes a preview of each. */
export class RttyScanner {
  constructor(inputSampleRate: number, options: RttyScannerOptions | null | undefined, onEvent: (event: RttyScanEvent) => void)
  /** Returns false (dropping nothing) when the queue is full; await drain() and retry. */
  pushF32(input: Float32Array): boolean
  /** Forgets every channel, in order with queued audio. */
  reset(): boolean
  drain(): Promise<void>
  finish(): Promise<void>
  /** Cancels queued work (its promises reject with RASTERWAVE_DISPOSED); every dispose() call resolves once the scanner is released. */
  dispose(): Promise<void>
  readonly queuedSamples: number
}
