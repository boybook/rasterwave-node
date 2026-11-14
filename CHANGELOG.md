# Changelog

## 0.6.0

- Upgrade to `rasterwave` 0.6.0 and expose its RTTY modem.
- `RttyEncoder`: synchronous interactive encoder (`pushText`, `retractUnsent`,
  `clearUnsent`, `finish`, `abort`, `readSamples`) that reports committed
  characters with sample positions.
- `RttyDecoder`: queued decoder with backpressure, `retune`, `setSquelch`,
  `updateSettings` (AFC, USOS, character set, lanes, auto-reverse),
  `markDiscontinuity(droppedInputSamples)`, a noise blanker option, character
  events with confidence and periodic signal reports (`snrDb` calibrated to a
  3 kHz bandwidth, per-tone `markDb`/`spaceDb` levels).
  - Control calls are validated in full before they are queued and throw
    `RASTERWAVE_INVALID_ARGUMENT` (including a `retune` whose shift is outside
    20..=1200 Hz and invalid `updateSettings` lanes or character set); a queued
    control command can no longer fail the session.
  - `markDiscontinuity` travels through the queue in order with the audio, so
    it never shifts audio queued before the call, is never lost when the queue
    is full, and `reset()` discards only gaps declared before it.
- `rttyProfiles()` lists the catalog profiles.
- `RttyScanner`: queued wideband RTTY signal browser with backpressure that
  reports tracked channels (`channels` every ~500 ms with stable ids, centre,
  shift, score, SNR, lock and polarity), per-channel preview text (`text`) and
  `removed` events. Events are discriminated on `type`, like the decoder
  events, and share their `drain`/`finished`/`error` control events.
- Calling `dispose()` again while a dispose is pending resolves every call.
- `preambleLtrs` and `tailLtrs` are documented as integers in 0..=32.

## 0.5.0

- Upgrade to `rasterwave` 0.5.0 with stable phasing-calibrated FAX rows.
- Remove the misleading per-line dead-sector geometry option and expose the
  conservative `imageContent` timing source for mid-image joins.
- Keep nominal-paper correction sparse, gated, and non-destructive; calibrated
  phasing captures are no longer retimed by later map content.

## 0.4.0

- Add opt-in enhanced SSTV calibration preamble, FSK-ID, CW station ID and
  end-guard silence through the streaming encoder.
- Expose encoder stage and exact raster sample boundaries for loopback preview.

## 0.3.1

- Upgrade to the joint radiofax phase/ppm tracker from `rasterwave` 0.3.1.

## 0.3.0

- Expose robust radiofax clock calibration, raster basis and synchronous clock snapshots.
- Add native asynchronous correction for nominal continuous-paper rasters.

## 0.2.0

- Add `continuousPaper` decoder backends for SSTV and radiofax while retaining
  the framed API.
- Expose monotonic raster rows, trusted boundaries, protocol completion ranges,
  and manual mismatch observations as discriminated events.
- Keep synchronous queue commands and native Promise barriers serialized by
  the existing per-instance FIFO actor.

## 0.1.1

- Add `immediateDecode` for forced SSTV reception in every built-in mode.
- Add `immediateDecode` for fixed IOC/LPM radiofax reception without waiting
  for APT or phasing.

## 0.1.0

- First stable npm release of the complete streaming SSTV and radiofax bridge.
- Verified native binaries on macOS arm64/x64, Linux glibc arm64/x64, and Windows x64.
- Added npm Trusted Publishing with GitHub Actions OIDC and registry provenance.

## 0.0.1

- Bootstrap release used to establish the npm packages and Trusted Publisher configuration.
- Native FIFO execution, bounded decoder queues, row callbacks, and Promise-based barriers.
- Prebuilt support for macOS arm64/x64, Linux glibc arm64/x64, and Windows x64.
