# Iwaks — Design Document (v1)

> Music player desktop Windows untuk audio hi-res & lossless offline, ala Poweramp Android.
> Stack: **Tauri v2 + Rust + libmpv → WASAPI + SQLite + React/TS/Tailwind**.

---

## 1. Understanding Summary

1. **Apa:** Aplikasi desktop Windows — music player hi-res & lossless offline ala Poweramp, dark modern, album art besar.
2. **Mengapa:** Pengalaman mendengarkan musik lokal berkualitas tinggi di PC.
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
| A7 | libmpv di-bundle sebagai `libmpv-2.dll` untuk Windows (supplied binary) |
| A8 | Urutan casting v1: **DLNA dulu → Chromecast menyusul** dalam siklus v1; Bluetooth post-v1 |

## 3. Decision Log

| # | Keputusan | Alternatif | Alasan |
|---|---|---|---|
| D1 | Pakai sendiri + share GitHub | — | Per user |
| D2 | Windows saja | macOS / Linux | WASAPI; fokus |
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
| Output | `ao=wasapi` **shared mode (default)** — exclusive mode membuat semua aplikasi lain senyap selama sesi, jadi shared dipilih; exclusive tetap tersedia via opsi (deviasi — lihat Amendemen M3 slice 5) |
| Gapless | Bawaan libmpv (on) |
| Crossfade | Dua instance libmpv + volume ramp — toggle, default off |
| EQ | `af=lavfi[equalizer]` 10 band + preamp + bass/treble |
| ReplayGain | `replaygain=track` (option album) |
| Volume | App-level 0–100, kurva non-linear |
| Resampler | `audio-resampler=soxr` (A1) |
| DSD | DSD-over-PCM via WASAPI bila device mendukung; fallback PCM (soxr) |

Event/command Tauri: `play`, `pause`, `seek`, `next/prev`, `set_volume`, `set_eq`, `set_replaygain`, `set_output_device`, `set_shuffle`, `reshuffle_tracks`, `add_files`, `toggle_mini_visualizer`; events keluar `track-change`, `position`, `playback-ended`, `device-list-changed`, `error`.

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
- **Amendemen M1:** M1 mengimplementasikan `tracks` + `tracks_fts` (external-content + trigger); grup album/artist tetap **diturunkan frontend-side** (tanpa join table — lihat Amendemen M3 slice 8); tabel `playlists`/`playlist_tracks` diimplementasikan di Amendemen M4 slice 1 — migrasi per versi `user_version`. Format diambil dari ekstensi file; cover art thumbnail ditampilkan on-demand (`read_cover` → data URL, cache sesi frontend), cache disk `cover_path` menyusul di M3/M4. Scan folder kedua **menumpuk** (multi-root); pemangkasan hanya untuk file yang benar-benar hilang dari disk (`clean_missing` = cek `exists`), bukan "di luar root terakhir". Scanner salah-baca file (corrupt) → dihitung `errors`, tidak crash, di-*retry* tiap scan (A6).
- **Amendemen M2:** Playback diimplementasikan di crate `crates/player` (menggantikan rencana `audio`): `ffi.rs` = binding libmpv 10 simbol (`Mpv` Send+Sync, tanpa lock internal). **Pump satu thread OWNS seluruh lifecycle libmpv** (create → options → init → `wait_event` → commands); tidak ada thread lain yang menyentuh handle — `mpv_wakeup` pun tidak dipakai, karena cross-thread call yang balapan dengan command sinkron dapat deadlock pada libmpv 0.41. Semua perintah publik (`play_tracks`, `toggle_play`, `seek`, `next/prev`, volume, mute) hanya push `Cmd` ke `Mutex<VecDeque>`; pump mengeksekusi di iterasi berikutnya (≤100 ms, tanpa wakeup). `on_end_file` **tidak pernah menyentuh libmpv** (read properti saat transisi EndFile→idle dapat self-deadlock core): arm Eof/Error/Redirect hanya mengantre `Cmd::Load` via `advance_to`; arm Stop (manual) hanya update state. Watchdog: di pump, semua lock queue **di-ikat ke `let` dulu** sebelum memanggil `play_index` — guard sementara pada scrutinee `match`/`if let` hidup selama ekspresi penuh → `play_index` yang mengunci ulang queue akan self-deadlock (bug nyata di `next/prev` yang ditemukan & diperbaiki di M2). Headless test: `ao=null` timed + `gapless=yes`; `playback-time` hanya observable pada file **≥ 1 dtk** (file lebih pendek ter-buffer penuh ke AO sehingga posisi pin di nilai awal — roundtrip memakai track 2 dtk). DLL via `scripts/fetch-libmpv.ps1` → `src-tauri/libmpv/libmpv-2.dll`; headless test di CI memakai DLL nyata dengan env `IWAKS_LIBMPV`.
- **Amendemen M3 (slice 1 — speed + sleep timer):** `speed` = properti libmpv, di-set via `Cmd::SetSpeed` → `set speed <v>`, di-clamp 0.25–4.0 di public API (setara `set_volume`); state baru `speed` (default 1.0) dibaca tiap tick. Sleep timer hidup di Rust sebagai source of truth: `sleep_deadline: Mutex<Option<Instant>>`; pump memeriksa deadline tiap loop → saat lewat, deadline dikosongkan dan `Cmd::SleepElapsed` diantre → `set pause yes` (hanya *pause*: queue tetap termuat sehingga resume sekali tekan; `stopped` tidak di-set). State baru `sleep_remaining: Option<f64>` = sisa detik (0 saat lewat-tapi-belum-dieksekusi), hilang saat timer habis/dibatalkan. Batal via `set_sleep_timer(None)` atau nilai ≤0 (nilai non-finite diabaikan). Command Tauri baru: `set_speed`, `set_sleep_timer`. Test headless: roundtrip `speed` + clamp atas/bawah, sleep timer mem-pause, cancel membuat timer tidak mem-pause.
- **Amendemen M3 (slice 2 — EQ + ReplayGain):** EQ memakai `af=lavfi[equalizer]` sesuai §4.2: 10 band ISO (31…16k Hz, lebar 1 oktaf, `t=o`) + preamp sebagai *master gain* (`volume` filter di akhir chain). Chain dibangun oleh fungsi murni `eq_af_chain(preamp, gains) -> Option<String>` (flat = `None` → `set af ""` — terverifikasi di probe mpv 0.41 bahwa string kosong benar-benar membersihkan chain); nilai di-clamp ±12 dB & dibulatkan 0,1 dB, band yang hilang dipad nol. Bass/treble pada §4.2 dicakup oleh band ujung (31/62 & 8k/16k) — shelf filter terpisah tidak dibuat (penyederhanaan, dokumentasi deviasi). `set_eq` menyimpan nilai di `Mutex<EqSettings>` (source of truth frontend) lalu set `af`; state baru `eq_preamp` + `eq: Vec<f64>`. ReplayGain: opsi startup `replaygain=track` (+ `preamp=0`, `clip=yes`, `fallback=0`) sesuai §4.2; mode bisa diganti runtime (`set replaygain <mode>`) dan **dibaca balik tiap tick** via properti string (ffi baru `get_string` + binding `mpv_free`). Contoh nyata mpv: nilai off adalah **`no`**, bukan `off` (`--replaygain=<no|track|album>`) — `ReplayGainMode` serde lowercase (`off|track|album`) dipetakan ke `no|track|album` saat ke mpv. Perubahan mode berlaku mulai file berikutnya dimuat. Command Tauri: `set_eq`, `set_replaygain`. Test: chain builder (flat/1 band/preamp/clamp) + roundtrip mode RG + EQ apply→clear tanpa mengganggu playback.
- **Amendemen M3 (slice 3 — lirik):** modul `crates/tags/src/lyrics.rs`. Embedded lyrics dibaca via `lofty` `ItemKey::Lyrics` — satu key yang memetakan USLT (ID3), `LYRICS` (Vorbis), dan `©lyr` (iTunes), jadi satu jalur baca untuk semua format yang didukung; sinkronisasi waktu **tidak** tersedia di embedded (lofty 0.22 tidak punya SYLT), jadi `.lrc` sidecar (`path.with_extension("lrc")`) adalah sumber timed. Prioritas: baris *timed* dari sidecar `.lrc` menang; bila tak ada sidecar, teks embedded yang mengandung timestamp di-auto-parse sebagai LRC; teks *plain* = embedded bila ada, fallback konten sidecar mentah. `parse_lrc` fungsi murni: `[mm:ss]`, `[mm:ss.xx]`, `[hh:mm:ss.xx]`, multi-timestamp per baris, `[offset:±ms]` diterapkan ke semua timestamp (clamp ≥ 0), tag metadata (`[ti:]`, `[ar:]`, …) dilewati, hasil diurutkan naik. Command Tauri `get_lyrics(path)` → `Option<Lyrics { timed, plain }>` (camelCase, dipanggil on-demand saat panel dibuka — tanpa state baru di player). Frontend: tombol lirik di player bar → popover (seperti EQ); baris aktif = timestamp terakhir ≤ posisi playback (dari tick state, ~10 Hz), auto-scroll smooth ke tengah, jatuh ke tampilan teks polos saat tak ada timed. Test (9 baru, fixture FLAC Vorbis-comment seperti cover.rs): parse timestamp/offset/negatif-clamp/metadata-dilewati, embedded `LYRICS` terbaca, sidecar menang atas embedded + fallback plain, embedded-LRC diparse untuk timing, tanpa lirik → `None`.
- **Amendemen M3 (slice 4 — visualizer, ditulis ulang):** crate `crates/visualizer` (symphonia + rustfft) **dihapus** dari workspace (Cargo.toml + src-tauri dep ikut dibersihkan). Visualizer kini **generatif bawaan** (`src/Visualizer.tsx`): canvas digambar dari kode, **bukan dari decode track** — tanpa FFT, tanpa decode paralel. Seeder `fnvhash(path) -> u32` (FNV-1a atas path file) membuat pola bar konsisten per lagu; 48/64 bar, denyut ~114 BPM, smoothing attack 0,45 / release 0,08, gradient panel + mini. **Mini window:** jendela frameless `label="mini"` (always-on-top, skip-taskbar, resizable) memuat `index.html#mini` → `src/MiniViz.tsx` (visualizer + kontrol, ✕ = `getCurrentWindow().close()`); **terbuka otomatis saat window utama di-minimize** (setup `on_window_event`: `Resized` → `sync_mini` via `is_minimized()`, `Destroyed` → tutup mini + `exit(0)`) + tombol toggle manual di player bar. Command Tauri baru: `open_mini_window`, `sync_mini`, `toggle_mini_visualizer`; `get_spectrum`/`Spectrum` dihapus (frontend `types.ts`/`api.ts` ikut dibersihkan). Gate: `npm run build` (murni frontend, tanpa Rust crate baru).
- **Amendemen M3 (slice 5 — audio, bug shared mode):** default `Options.audio_exclusive` di `crates/player/src/player.rs` diubah **`true` → `false`** → WASAPI **shared** — **deviasi dari §4.2 "bit-perfect"**: exclusive mode mengunci perangkat sample-accurately sehingga app lain (Discord/YouTube) senyap selama sesi, yang mengejutkan user. Shared = default; exclusive tetap bisa diaktifkan via opsi. Test: `defaults_use_shared_wasapi_audio`.
- **Amendemen M3 (slice 6 — tambah musik):** `scan_files(lib, paths, …)` di `crates/library/src/scan.rs` (tambah file tertentu — multi-select dialog; tanpa rekursi/pruning) + command Tauri `add_files`; frontend `pickFiles` (multi-file via plugin dialog) + tombol "Add files" + **refresh otomatis daftar setelah scan/add** lewat `refreshKey` (dibump di `listenScan` finished & `onAddFiles` — perbaikan "list yang harus diperbaiki"). Scan folder (`scan_folder`) menumpuk multi-root seperti desain. Kunci dedupe `norm()` kini menyamakan `/` dan `\` (path dialog bisa campur separator di Windows) + case-fold, sehingga re-add/re-scan collapse ke baris yang sama (`ON CONFLICT(path)`).
- **Amendemen M3 (slice 7 — shuffle + reshuffle):** queue (`crates/player/src/queue.rs`) memakai **permutasi order** deterministik (SplitMix64, tanpa dep rand): mengaktifkan shuffle mem-pin track saat ini ke **posisi 0** permutasi → "next" berjalan melewati seluruh daftar (tidak melompat ke tengah saat toggle di tengah lagu); `reshuffle(seed)` mengocok ulang sisa (posisi saat ini tetap); matikan shuffle → identity. `next_shuffle_seed` (AtomicU64, start `0xDEADBEEF`, step `0x9E3779B97F4A7C15`) bertambah tiap lagu; shuffle dipertahankan lewat `play_tracks`. State baru `shuffle` di `PlayerState` (di-*readback*); command Tauri: `set_shuffle`, `reshuffle_tracks`; UI: toggle shuffle + tombol dice (reshuffle). Test headless: roundtrip pin-current/reshuffle/persist (pump memegang libmpv, handler touch-queue tidak memanggil `play_index`).
- **Amendemen M3 (slice 8 — navigasi library):** sidebar Songs/Albums/Artists/Folders menjadi view nyata (`view` state di `App.tsx`); Albums/Artists/Folders **diturunkan frontend-side** dari `getTracks()` (`groupBy` — grup album/artist/folder, kartu grid memakai cover track pertama, urutan alphabet case-insensitive); klik kartu → daftar lagu grup (play dari posisi grup, filter search tetap berlaku), tombol back + judul breadcrumb (`back-btn`); link Playlists **ditunda** (keputusan user) dan dihapus dari nav. `refreshKey` juga mem-perbarui grid setelah scan/add.
  - **Amendemen M4 (slice 1 — playlist):** tabel `playlists(id, name, m3u_path)` + `playlist_tracks(playlist_id, track_id, position)` di `SCHEMA_V2` — migrasi `user_version` 0→2 & 1→2; `PRAGMA foreign_keys = ON` di `Library::open`, PK `(playlist_id, track_id)` men-dedupe entri, FK `ON DELETE CASCADE` membersihkan entri saat playlist/track dihapus (termasuk pruning scan). `crates/library/src/playlists.rs`: `create_playlist` (trim, blok kosong), `rename_playlist`/`delete_playlist` (idempoten; missing → `NotFound`), `list_playlists`/`get_playlist` (dengan `track_count`), `get_playlist_tracks` (urut `position`), `add_track_to_playlist` (append + dedupe), `remove_track_from_playlist`, `reorder_playlist` (satu transaksi; daftar = urutan baru, entri tak terdaftar di-drop). `.m3u`: `export_m3u` (`#EXTM3U` + `#EXTINF:<dur>,<title>` + path), `import_m3u` (nama = file stem; baris kosong/`#` dilewati; path relatif di-resolve dari direktori file; pencocokan path memakai `norm()` separator-agnostic; entri yang tak ter-resolve ke track library **di-skip** — playlist tetap dibuat). 10 command Tauri baru: `list_playlists`, `create_playlist`, `rename_playlist`, `delete_playlist`, `get_playlist_tracks`, `add_to_playlist`, `remove_from_playlist`, `reorder_playlist`, `import_m3u`, `export_m3u`. Frontend: nav **Playlists** kembali (grid kartu: nama + jumlah lagu; "New playlist" inline; "Import .m3u"), detail playlist (back, rename inline, export via save dialog, delete + konfirmasi, tombol **✕** per baris, play dari posisi, search mem-filter isi playlist), dan tombol **+** di tiap baris lagu (Songs/detail grup) → panel picker playlist + "Create & add" (playlist baru + tambah sekaligus). Test library 12 baru (CRUD, blank name, dedupe/urutan, cascade dua arah, reorder/truncate, m3u roundtrip, skip entri hilang, path relatif, migrasi 1→2).
  - **Amendemen M4 (slice 2 — queue sesi):** view antrean membaca urutan *aktual yang akan diputar* dari player. `Queue` (pure, `queue.rs`) mendapat `reorder(from, to)` — pindahkan track di posisi `from` ke `to` (di-clamp ke slot terakhir; `from` di luar rentang / queue kosong = no-op) dan `index` di-reposisi mengikuti track yang sedang berjalan (`pos == from → to`, slot antara bergeser ±1), sehingga `current()` ***tidak pernah berubah*** — playback tak tersentuh, hanya urutan ke depan yang diedit; `shuffle` tetap menyala dan `order` tetap permutasi (anggap sebagai override manual). Getter baru `ordered_indices()` mengekspos urutan tanpa membuka field privat. `Player`: `queue_tracks()` (snapshot `Vec<Track>` urutan play — lock order queue→tracks konsisten dengan `play_index`/`read_state`; hanya baca, tanpa sentuh mpv) dan `reorder_queue(from, to)` (touch queue saja + `Cmd::Refresh` emit state — pola sama dengan `set_repeat`). Save-as-playlist: `add_tracks_to_playlist(playlist_id, track_ids)` di library — **satu transaksi**, `INSERT OR IGNORE` (dedupe via PK) dengan `position = max(position)+1` per baris, gagal satu entri (FK track tak dikenal) → **rollback penuh**; mengembalikan jumlah yang benar-benar ter-insert. 3 command Tauri baru: `get_queue` (urut play order), `reorder_queue`, `save_queue_as_playlist` (ambil `queue_tracks()`, error bila kosong, lalu `create_playlist` + bulk add, return id). Frontend: nav **Queue** — daftar urutan play (nomor posisi, cover, indikator **Now playing** dengan dot berdenyut, handle grip), **drag & drop** (HTML5 DnD): drag baris → kasar di slot → `reorder_queue(from, to)` (baris mendarat di slot yang di-hover; baris sumber diredupkan, garis aksen atas sebagai penanda jatuh; drop di luar baris dibatalkan). Tombol **Save as playlist** di toolbar → `InlineName` → langsung buka detail playlist baru. Queue di-refetch ketika `index`/`listLen`/`shuffle` berubah di event `player-state` (bukan tiap tick); search **tidak** mem-filter queue (indeks harus stabil agar reorder konsisten). Reorder keyboard di luar scope slice ini. Test: 7 pure `Queue::reorder`/`ordered_indices`, 2 headless player (reorder mengubah urutan next-up tanpa mengubah lagu berjalan; reorder konsisten dengan permutasi shuffle), 4 library bulk-add (urutan+dedupe, list kosong no-op, playlist missing → `NotFound`, rollback saat FK gagal).
- Folder browse: view langsung struktur folder, baca tag on-the-fly + cache tipis.
- Tag editor: FLAC (Vorbis), MP3 (ID3v2), M4A, OGG, WavPack, AIFF, DSF/DFF; backup `.bak` sebelum tulis; via `lofty`.
- Playlist: internal SQLite + impor/ekspor `.m3u`; queue sesi (drag-reorder, save-as-playlist).

### 4.4 UI/UX (frontend)

- Layout: sidebar (Songs/Albums/Artists/Folders — Playlists ditunda) + konten + player bar selalu tampak + Now Playing (art besar, lirik, visualizer).
- Prinsip skill: kontras WCAG AA, semua state ada (loading/empty/error), motion `cubic-bezier(0.23,1,0.32,1)` + `prefers-reduced-motion`, anti-template generik, art sebagai elemen hero.
- Visualizer: **generatif bawaan** (canvas + FNV hash path → pola per lagu, denyut ~114 BPM) — tanpa decode/FFT; panel di player bar + **mini window** auto-buka saat minimize (see Amendemen M3 slice 4).
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
- `player`: unit + integrasi headless dengan **libmpv DLL nyata** (`ao=null` timed, `IWAKS_LIBMPV` → jala penuh pompa, auto-skip korup, roundtrip play/pause/seek/volume/next/wrap).
- `cast`: trait + mock di CI.
- Frontend: vitest + testing-library (queue reorder, filter).
- Manual: WASAPI exclusive di perangkat asli, cast ke renderer sungguhan.

### 4.8 Milestone

| M | Isi | Keluar |
|---|---|---|
| M0 | Scaffold Tauri + workspace + CI + benchmark baseline | Repo jalan, `npm run tauri dev` |
| M1 | `core` + `library` + scanner | Lagu muncul, list + search |
| M2 | `player` libmpv (pump single-thread) + player bar | Play/pause/seek/volume/next/prev + bar |
| M3 | EQ, ReplayGain, sleep timer, speed, visualizer, lirik | Slice 1 (speed + sleep timer) ✅, Slice 2 (EQ + ReplayGain) ✅, Slice 3 (lirik) ✅, Slice 4 (visualizer generatif + mini window) ✅ — **M3 lengkap**; batch post-M3: shared-WASAPI default, add files/folder, shuffle + reshuffle, nav Albums/Artists/Folders (Amendemen M3 slice 5–8) ✅ |
| M4 | Playlist + queue + folder browse + tag editor | Slice 1 (playlist + m3u import/export) ✅, Slice 2 (queue: view + drag-reorder + save-as-playlist) ✅ — folder browse, tag editor menyusul |
| M5 | Casting DLNA → Chromecast + NSIS installer + README GitHub | Rilis v1 |

## 5. Risiko Kunci

- **Casting**: implementasi UPnP/DLNA (SOAP) dan CASTV2 dari nol di Rust — paling berisiko; dimitigasi dengan memulai DLNA dulu, trait terisolasi.
- **Visualizer**: sejak dirombak jadi **generatif** (tanpa tap PCM / decode), risiko teknis decode paralel symphonia hilang — lihat Amendemen M3 slice 4.
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