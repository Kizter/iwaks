# 🎵 MusicForge

**Hi-res & lossless offline music player for Windows** — a Poweramp-like experience for the desktop.

Built with **Tauri v2 (Rust) + React/TypeScript**. Audio engine: **libmpv → WASAPI (bit-perfect)**.

## Features (roadmap)

- ✅ **M0** — Scaffold, CI, benchmark baseline *(current)*
- ⏳ **M1** — Library core, SQLite scanner, search
- ⏳ **M2** — Playback (libmpv), player bar
- ⏳ **M3** — EQ, ReplayGain, sleep timer, playback speed, visualizer, lyrics
- ⏳ **M4** — Playlists, queue, folder browse, tag editor
- ⏳ **M5** — Casting (DLNA → Chromecast), installer, release

Format support (target): FLAC, WAV, ALAC, MP3, AAC, OGG, Opus, WavPack, AIFF, WMA Lossless, DSD (DSF/DFF).

Full design: [`docs/design.md`](docs/design.md)

## Prerequisites

- [Rust](https://rustup.rs) (stable, MSVC toolchain)
- [VS 2022 Build Tools](https://visualstudio.microsoft.com/downloads/) — workload "Desktop development with C++"
- Node.js ≥ 22
- WebView2 runtime (bawaan Windows 11)

## Development

```bash
npm install
npm run tauri dev
```

Build installer (NSIS):

```bash
npm run tauri build
```

## Tests & Quality

```bash
npm run build          # tsc + vite
cd src-tauri && cargo test && cargo clippy && cargo fmt --check
```

## Benchmarks

Build/CI timings are tracked in [`.ecc/benchmarks/build.json`](.ecc/benchmarks/build.json)
via `scripts/benchmark.ps1` — run `./scripts/benchmark.ps1 baseline` to refresh.

## License

MIT