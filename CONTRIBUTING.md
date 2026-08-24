# Contributing

Run the full local validation before opening a pull request:

```bash
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test
npm run build:debug
npm run check
```

Keep protocol and DSP behavior in the upstream `rasterwave` crate. This
repository owns Node-API conversion, native scheduling, packaging, and API
contract tests.
