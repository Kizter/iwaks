# Iwaks — Design Document (v1)

> Music player desktop Windows untuk audio hi-res & lossless offline, ala Poweramp Android.
> Stack: **Tauri v2 + Rust + libmpv → WASAPI exclusive + SQLite + React/TS/Tailwind**.

---

## 1. Understanding Summary

1. **Apa:** Aplikasi desktop Windows — music player hi-res & lossless offline ala Poweramp, dark modern, album art besar.
2. **Mengapa:** Pengalaman mendengarkan musik lokal berkualitas tinggi (bit-perfect) di PC.
3. **Untuk siapa:** Pemakaian pribadi, lalu dibagikan publik via GitHub (open source, bisa di-install orang lain).
4. **Kendala:** Windows saja • Tauri v2 + Rust • libmpv → WASAPI • SQLite (skala medium 1.000–20.000 lagu).
5. **Format audio:** Semua — FLAC/WAV/ALAC, MP3/AAC/OGG/Opus, WavPack/AIFF/WMA Lossless, DSD (DSF/DFF).
6. **Fitur inti:** Scanner folder + metadata tag, pencarian + filter, playlist + queue, EQ, gapless + crossfade + ReplayGain, sleep timer + playback speed, visualizer, lirik, tag editor, folder browse, casting (DLNA → Chromecast; Bluetooth post-v1).
7. **Non-goals:** Tanpa streaming/online (aplikasi offline murni), tanpa remote control jarak jauh.

## 2. Assumptions

| # | Asumsi |
|---|--------|
| A1 | Resampler berkualitas tinggi (soxr) aktif melalui libmpv — hampir gratis, fallback saat device tidak mendukung sample rate file |
| A2 | Performance target: warm start < 3 dtk, scan 10.000 file < 30 dtk, RAM < 300 MB, UI responsif saat lagu hi-res diputar |
| A3 | Privasi: offline murni, tanpa telemetri; koneksi jaringan hanya saat casting/scan | foldern |
| A4 | Bahasa UI: Inggris (netral untuk berbagi GitHub); nama app "Iwaks" (final) |
| A5 | Lisensi GitHub: MIT |
| A6 | Reliability: file corrupt/format aneh → skip + warning, tidak pernah crash |
| A7 | libmpv di-bundle sebagai `mpv-1.dll` untuk Windows (supplied binary) |
| A8 | Urutan casting v1: **DLNA dulu → Chromecast menyusul** dalam siklus v1; Bluetooth post-v1 |

## 3. Decision Log

| # | Keputusan | Alternatif | Alasan |
|---|---|---|---|
| D1 | Pakai sendiri + share GitHub | — | Per user |
| D2 | Windows saja | macOS / Linux | WASAPI exclusive; fokus |
| D3 | Semua format audio | subset | Per user |
| D4 | EQ + gapless/crossfade/ReplayGain + sleep timer/speed | resampler terpisah | Per user; resampler = A1 |
| D5 | Scanner+tag, search+filter, playlist+queue | — | Per user |
| D6 | Tag editor + folder browse masuk scope v1 | non-goal | Koreksi user |
| D7 | Dark modern ala Poweramp → **final: terang krem + tinta biru** (konsep sketsa user) | minimalis / klasik | Pilihan user; dikunci lewat konsep gambar |
| D8 | Tauri v2 + Rust | Electron, C# | Ringan, WASAPI, ekosistem |
| D9 | libmpv engine | Pure Rust decode | Semua format + DSD, matang |
| D10 | Pendekatan A: modular monolith | B (thin frontend), C (proses terpisah) | Scope besar, satu binary |
| D11 | SQLite (rusqlite) | JSON / Postgres | Skala medium |
| D12 | Casting: BT+DLNA dulu, Chromecast menyusul | casting penuh sekaligus | Risiko teknis |
| D13 | Skill frontend: impeccable, ui-ux-pro-max, apple-design, design-taste-frontend, accessibility, anti-ui-slop | — | Per user |
| D14 | Skill backend: tailwind-patterns, typescript-expert, backend-architect, tauri-v2 (+ rust-architect opsional) | — | Per user |
| D15 | Skill saat kerja: clean-code, ponytail, performance-optimizer, benchmark | — | Per user |
| D16 | **Bluetooth ditunda post-v1** | — | Koreksi user |
| D17 | **Branding: nama "Iwaks", maskot ikan koi+caset, tema terang krem+biru** | — | Konsep user (D7 diubah) |

## 4. Final Design

### 4.1 Arsitektur — Cargo workspace (modular monolith)

```
iwaks/
├── crates/
│   ├── core/           # Domain murni: Track, Album, Playlist, queue logic (tanpa I/O)
│   ├── audio/          # Wrapper libmpv: playback, EQ, volume, events, output devices
│   ├── library/        # SQLite (rusqlite), scanner background, search FTS5, migrasi
│   ├── tags/           # Baca/tulis tag via lofty (tag editor + backup/rollback)
│   ├── visualizer/     # Decode paralel ringan (symphonia) → FFT (rustfft) → spektrum
│   ├── cast/           # Trait CastTarget: UPnP/DLNA, CASTV2 (nanti)
│   └── app/            # Binary Tauri: komposisi state, IPC commands, events
└── src/ (frontend)
    ├── React + TS + Tailwind
    └── views: Library, FolderBrowse, NowPlaying, EQ, Cast, Playlists
```

- **Satu arah data:** Rust core = source of truth; frontend = tampilan + intent.
- Rasul: core murni pure-logic (testable); semua I/O jahat (file, jaringan) di crate masing-masing.
- Immutability di domain: objek dibuat baru, tidak dimutasi.

### 4.2 Pipeline Audio (libmpv)

| Aspek | Keputusan |
|---|---|
| Output | `ao=wasapi` exclusive mode (bit-perfect, buka perangkat sesuai sample rate file) |
| Gapless | Bawaan libmpv (on) |
| Crossfade | Dua instance libmpv + volume ramp — toggle, default off |
| EQ | `af=lavfi[equalizer]` 10 band + preamp + bass/treble |
| ReplayGain | `replaygain=track` (option album) |
| Volume | App-level 0–100, kurva non-linear |
| Resampler | `audio-resampler=soxr` (A1) |
| DSD | DSD-over-PCM via WASAPI bila device mendukung; fallback PCM (soxr) |

Event/command Tauri: `play`, `pause`, `seek`, `next/prev`, `set_volume`, `set_eq`, `set_replaygain`, `set_output_device`; events keluar `track-change`, `position`, `playback-ended`, `device-list-changed`, `error`.

Edge cases: file corrupt → toast + auto-skip; device hilang → pause + notifikasi; rate mismatch → soxr.

### 4.3 Library — SQLite

```
tracks(id, path UNIQUE, title, artist, album, album_artist, genre, year,
       track_no, disc_no, duration_ms, sample_rate, bit_depth, bitrate,
       format, file_size, modified_at, cover_path, replaygain)
albums(id, title, artist, year, cover_path)
artists(id, name)
playlists(id, name, m3u_path)
playlist_tracks(playlist_id, track_id, position)
```
- FTS5 untuk pencarian; scanner **inkremental** (hanya file dengan `modified_at` berubah).
- **Amendemen M1:** M1 mengimplementasikan `tracks` + `tracks_fts` (external-content + trigger); tabel `albums`/`artists`/`playlists` menyusul di M4 (UI browse) — migrasi per versi `user_version`. Format diambil dari ekstensi file; cover art thumbnail ditampilkan on-demand (`read_cover` → data URL, cache sesi frontend), cache disk `cover_path` menyusul di M3/M4. Scan folder kedua **menumpuk** (multi-root); pemangkasan hanya untuk file yang benar-benar hilang dari disk (`clean_missing` = cek `exists`), bukan "di luar root terakhir". Scanner salah-baca file (corrupt) → dihitung `errors`, tidak crash, di-*retry* tiap scan (A6).
- Folder browse: view langsung struktur folder, baca tag on-the-fly + cache tipis.
- Tag editor: FLAC (Vorbis), MP3 (ID3v2), M4A, OGG, WavPack, AIFF, DSF/DFF; backup `.bak` sebelum tulis; via `lofty`.
- Playlist: internal SQLite + impor/ekspor `.m3u`; queue sesi (drag-reorder, save-as-playlist).

### 4.4 UI/UX (frontend)

- Layout: sidebar (Musik/Artis/Album/Genre/Playlist/Folder) + konten + player bar selalu tampak + Now Playing (art besar, lirik, visualizer).
- Prinsip skill: kontras WCAG AA, semua state ada (loading/empty/error), motion `cubic-bezier(0.23,1,0.32,1)` + `prefers-reduced-motion`, anti-template generik, art sebagai elemen hero.
- Visualizer: decode paralel symphonia untuk FFT (sinkron via timestamp) → 60–120 bin → event Tauri 30fps (throttle) → render canvas/SVG (bar + ring).
- Lirik: embedded USLT / Vorbis `LYRICS` / `.lrc` samping lagu; highlight sinkron.
- Kinerja: virtual list (hand-rolled fixed-row, menghindari dep react-window/peer-dep React 19; M1) utk daftar >500 baris, debounce search, lazy-load cover + cache disk.

### 4.5 Casting

- **DLNA (v1):** discovery SSDP → app sebagai MediaServer (HTTP) + Control Point; push stream + kontrol AVTransport; view Cast daftar renderer.
- **Chromecast (v1 lanjutan):** CASTV2 (mDNS + protobuf), mirror logika kontrol.
- **Bluetooth: post-v1 (D16).**
- Satu trait `CastTarget` → UI sama untuk semua target.

### 4.6 Error Handling & Reliability

- Tipe `AppError` terpusat → pesan UI + kode; tanpa `unwrap` di jalur user-facing.
- Panic hook → dialog + log `%LOCALAPPDATA%/Iwaks/logs`.
- Playback error → toast merah + auto-skip.

### 4.7 Testing (TDD, ≥80% saat ada test)

- `core`: unit test pure (queue, replaygain, filter).
- `library`: scanner atas fixture folder, migrasi DB.
- `tags`: round-trip baca/tulis di copy fixture (bukan file asli).
- `audio`/`cast`: trait + mock di CI.
- Frontend: vitest + testing-library (queue reorder, filter).
- Manual: WASAPI exclusive di perangkat asli, cast ke renderer sungguhan.

### 4.8 Milestone

| M | Isi | Keluar |
|---|---|---|
| M0 | Scaffold Tauri + workspace + CI + benchmark baseline | Repo jalan, `npm run tauri dev` |
| M1 | `core` + `library` + scanner | Lagu muncul, list + search |
| M2 | `audio` libmpv | Play/pause/seek/volume/gapless + player bar |
| M3 | EQ, ReplayGain, sleep timer, speed, visualizer, lirik | Serangkaian fitur audio selesai |
| M4 | Playlist + queue + folder browse + tag editor | Manajemen library lengkap |
| M5 | Casting DLNA → Chromecast + NSIS installer + README GitHub | Rilis v1 |

## 5. Risiko Kunci

- **Casting**: implementasi UPnP/DLNA (SOAP) dan CASTV2 dari nol di Rust — paling berisiko; dimitigasi dengan memulai DLNA dulu, trait terisolasi.
- **Visualizer PCM**: tidak ada API resmi tap PCM di libmpv → decode paralel symphonia (CPU ~5%); kebutuhan sinkron timestamp.
- **DSD**: perilaku tergantung device (native vs PCM fallback) — perlu pengujian perangkat nyata.
- **WASAPI exclusive**: koneksi bisa "hilang" saat device diputus — perlu listener device.

## 6. Brand & Identitas Visual

**Nama aplikasi:** Iwaks (dari "ikan" — maskot ikan).

**Tema:** terinspirasi estetika band indie asal Surabaya **Crayoncase** (noise pop/shoegaze):
- DIY lo-fi, playful "berbagai warna krayon", sentuhan indie Jepang (Supercar/Solanin era)
- Nostalgia kaset/CD, film photography, dreamy noise
- Diekspresikan sebagai: **tema terang krem + tinta biru** (dikonfirmasi user: konsep sketsa tangan — ikan + kaset + not musik; bukan dark) + detail hand-drawn/sketch

**Maskot (ikon resmi):** sketsa tinta biru — ikan koi dengan kumis, duduk di atas kaset, not musik melayang, bintang kecil; isian mint muda + highlight es. Dikonversi via `tauri icon` (ico/png/icns semua ukuran). Tetap terbaca di 16px (favicon) hingga 512px (app icon).

**Palet inti (v2 — terang):**
| Token | Hex | Penggunaan |
|---|---|---|
| `--bg` | `#f4f6ea` | Latar utama (krem) |
| `--surface` | `#ffffff` | Panel/card |
| `--ink` | `#026aa7` | Tinta biru — aksen/utama |
| `--mint` | `#e3f1e7` | Isian lembut / highlight lembut |
| `--ice` | `#c8f7ff` | Highlight (kilau mata/reel) |
| `--text` | `#26405c` | Teks utama |
| `--text-dim` | `#5f7d96` | Teks sekunder |

## 7. Open Items (bukan blocker)

- Skill `rust-architect` (nanlong) — 21 install, opsional; default pakai tauri-v2 + clean-code.
- Nama aplikasi final: Iwaks (sudah dikunci).
- Bahasa UI final: Inggris (A4).