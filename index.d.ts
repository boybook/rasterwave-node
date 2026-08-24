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
export interface SstvDecoderOptions {
  immediateDecode?: boolean
  detectVis?: boolean; detectSyncTiming?: boolean; manualMode?: SstvMode
  minimumSignalLevel?: number; queueCapacitySamples?: number
}
export interface SstvEncoderOptions { amplitude?: number; toneOffsetHz?: number; includeVisHeader?: boolean }

export type SstvDecodeEvent =
  | { type: 'modeCandidate'; candidates: SstvMode[]; confidence: number }
  | { type: 'imageStarted'; imageId: number; mode: SstvMode; detection: 'vis' | 'syncTiming' | 'manual' | 'unknown'; visCode?: number; ambiguous?: boolean; candidateCount?: number; frequencyOffsetHz: number; width: number; height: number }
  | { type: 'lineReady'; imageId: number; mode: SstvMode; lineIndex: number; revision: number; completeness: 'provisional' | 'final'; pixels: Uint8Array }
  | { type: 'imageCompleted'; imageId: number; mode: SstvMode; lines: number }
  | { type: 'imageAborted'; imageId: number; mode: SstvMode; lastLine?: number; reason: 'inputDiscontinuity' | 'endOfInput' | 'reset' | 'syncLost' | 'unknown' }
  | { type: 'signalRejected'; reason: string }
  | DecoderControlEvent | DecoderErrorEvent
export type DecoderControlEvent = { type: 'drain' | 'finished' }
export type DecoderErrorEvent = { type: 'error'; reason: string }
export interface EncoderProgress { samplesEmitted: number; estimatedTotalSamples: number; currentRow?: number; finished: boolean }

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
  trailingBlackSeconds?: number; deadSectorFraction?: number
}
export interface FaxEncoderOptions { amplitude?: number; includeApt?: boolean; includePhasing?: boolean }
export interface FaxDecoderOptions {
  immediateDecode?: boolean
  ioc?: FaxIoc; lpm?: number; modulation?: FaxModulationOptions; maxLines?: number
  amFullScale?: number; expectedPhasingSeconds?: number; aptConfirmSeconds?: number
  acquisitionTimeoutSeconds?: number; stopConfirmSeconds?: number; signalLossSeconds?: number
  minimumSignalLevel?: number; minimumCarrierCoherence?: number; queueCapacitySamples?: number
}
export type FaxDecodeEvent =
  | { type: 'aptDetected'; ioc: FaxIoc }
  | { type: 'phasingLocked'; ioc: FaxIoc; lpm: number; width: number }
  | { type: 'pageStarted'; pageId: number; ioc: FaxIoc; lpm: number; width: number; activeWidth: number; modulation: 'fm' | 'am' }
  | { type: 'lineReady'; pageId: number; lineIndex: number; pixels: Uint8Array }
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
}
export class FaxEncoder {
  constructor(pixels: Uint8Array, width: number, height: number, spec: FaxSpecOptions, sampleRate: number, options?: FaxEncoderOptions | null)
  readSamples(maxSamples: number): Promise<Float32Array>
  dispose(): Promise<void>
  readonly isFinished: boolean
  readonly progress: FaxEncoderProgress
}
export function sstvModes(): SstvModeInfo[]
