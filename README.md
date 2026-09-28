# 🐟 Iwaks

**Hi-res & lossless offline music player for Windows** — a Poweramp-like experience for the desktop. 

Built with **Tauri v2 (Rust) + React/TypeScript**. Audio engine: **libmpv → WASAPI** (shared mode default; exclusive opt-in).

## Features (roadmap)

- ✅ **M0** — Scaffold, CI, benchmark baseline, branding
- ✅ **M1** — Library core, SQLite scanner, FTS5 search, library UI
- ✅ **M2** — Playback (libmpv), player bar
- ✅ **M3** — speed + sleep timer, EQ + ReplayGain, lyrics, generative visualizer
- ✅ **Post-M3** — shared-WASAPI audio fix, add files / scan folder, shuffle + reshuffle, Albums / Artists / Folders browse, mini player window on minimize
- ✅ **M4** — Playlists (slice 1: CRUD + `.m3u` import/export, add/remove songs), queue (slice 2: Queue view, drag-reorder, save-as-playlist), folder browse (slice 3: hierarchical tree, drill-down, breadcrumb), tag editor (slice 4: 8-field modal, `.bak` backup, library sync), mini player (slice 5: mini window jadi player — draggable di mana saja, play/pause, prev/next, shuffle, ganti playlist/queue), UI polish (slice 6: visualizer dihapus, ikon flat Phosphor, menu sidebar + player bar diperbaiki), UI/UX overhaul (slice 7: palet warm pastel-muted + monokrom — parchment + dusty rosewood, sidebar berlabel + pill aktif + rail indikator, ikon search di toolbar, mini player diselaraskan + judul aktif, motion pass scaleX/0.95/shimmer transform, reduced-motion terpadu, scrollbar tipis) — **M4 complete** (+ pasca-uji: player bar responsif tanpa tumpang tindih ikon saat window diperkecil, panel playlist mini player muncul otomatis, mini player diselaraskan dengan tema utama + ikon Phosphor + ikon pada baris queue/playlist, window mini tetap ukuran — picker sebagai overlay, maskot app diganti + ikon window/taskbar/installer diregen dari artwork baru)
- ✅ **M5** — Release v1: NSIS installer (`npm run tauri build`), tag `v0.1.0`, docs release (`docs/design.md` Amendemen M5), window `backgroundColor` → palet baru (`#f3ede4`). Casting (DLNA → Chromecast) **ditunda post-v1** — keputusan evaluasi M5 (anti over-engineering: pemakaian pribadi, tanpa kebutuhan casting)

## Status: v1 (0.1.0)

App sudah layak disebut **ship untuk pemakaian pribadi**. Installer NSIS dihasilkan via `npm run tauri build` → `src-tauri/target/release/bundle/nsis/`. Casting/bluetooth/dark mode = post-v1 (lihat [`docs/design.md`](docs/design.md) §4.5 & milestone).

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
