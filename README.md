# 🐟 Iwaks

**Hi-res & lossless offline music player for Windows** — a Poweramp-like experience for the desktop. 

Built with **Tauri v2 (Rust) + React/TypeScript**. Audio engine: **libmpv → WASAPI** (shared mode default; exclusive opt-in).

## Features (roadmap)

- ✅ **M0** — Scaffold, CI, benchmark baseline, branding
- ✅ **M1** — Library core, SQLite scanner, FTS5 search, library UI
- ✅ **M2** — Playback (libmpv), player bar
- ✅ **M3** — speed + sleep timer, EQ + ReplayGain, lyrics, generative visualizer
- ✅ **Post-M3** — shared-WASAPI audio fix, add files / scan folder, shuffle + reshuffle, Albums / Artists / Folders browse, mini visualizer window on minimize
- 🚧 **M4** — Playlists ✅ (slice 1: CRUD + `.m3u` import/export, add/remove songs), queue ✅ (slice 2: Queue view, drag-reorder, save-as-playlist), folder browse, tag editor
- ⏳ **M5** — Casting (DLNA → Chromecast), installer, release

Format support (target): FLAC, WAV, ALAC, MP3, AAC, OGG, Opus, WavPack, AIFF, WMA Lossless, DSD (DSF/DFF).

Full design: [`docs/design.md`](docs/design.md)

## Prerequisites

- [Rust](https://rustup.rs) (stable, MSVC toolchain)
- [VS 2022 Build Tools](https://visualstudio.microsoft.com/downloads/) — workload "Desktop development with C++"
- Node.js ≥ 22
- WebView2 runtime (bawaan Windows 11)
- libmpv DLL — fetch once via `powershell -ExecutionPolicy Bypass -File scripts/fetch-libmpv.ps1` (downloads `src-tauri/libmpv/libmpv-2.dll`)

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
