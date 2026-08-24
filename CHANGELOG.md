# Changelog

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
