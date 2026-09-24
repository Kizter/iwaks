import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { getTracks, listenScan, pickFolder, readCover, scanFolder, searchTracks } from "./api";
import "./App.css";
import { formatBadge, formatDuration, scanSummary } from "./format";
import type { ScanProgress, Track } from "./types";
import iwaksMark from "./assets/iwaks-mark.png";

const ROW_HEIGHT = 56;

/** Embedded artwork per track path, cached for the session (null = has none). */
const coverCache = new Map<string, string | null>();

/** Music-note fallback shown when a track has no embedded cover art. */
function NoteIcon() {
  return (
    <svg className="cover-note" viewBox="0 0 16 16" width="18" height="18" aria-hidden="true">
      <path
        fill="currentColor"
        d="M7.5 2v9.2A2.1 2.1 0 1 0 9.1 13.1V3.1l4.4-1.03v7.5a2.1 2.1 0 1 0 1.6 2.06V1.3L7.5 2z"
      />
    </svg>
  );
}

/** Track thumbnail: embedded art when present, else a colored note tile. */
function Cover({ track }: { track: Track }) {
  const [src, setSrc] = useState<string | null | undefined>(undefined); // undefined = loading
  const fmt = track.format.toLowerCase();

  useEffect(() => {
    let alive = true;
    if (coverCache.has(track.path)) {
      setSrc(coverCache.get(track.path) ?? null);
      return;
    }
    readCover(track.path)
      .then((u) => {
        coverCache.set(track.path, u);
        if (alive) setSrc(u);
      })
      .catch(() => {
        coverCache.set(track.path, null);
        if (alive) setSrc(null);
      });
    return () => {
      alive = false;
    };
  }, [track.path]);

  return (
    <span className={`cover cover-${fmt}`} aria-hidden="true">
      {src ? <img className="cover-art" src={src} alt="" loading="lazy" /> : <NoteIcon />}
    </span>
  );
}

/** Debounced search-as-you-type hook. Blank query → full library. */
function useSearch(query: string) {
  const [tracks, setTracks] = useState<Track[] | null>(null);
  useEffect(() => {
    let alive = true;
    const timer = setTimeout(() => {
      const run = query.trim() === "" ? getTracks() : searchTracks(query);
      run.then((t) => {
        if (alive) setTracks(t);
      });
    }, 180);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [query]);
  return tracks;
}

function TrackRow({ track, top }: { track: Track; top: number }) {
  const artist = track.artist ?? "Unknown artist";
  const album = track.album ?? "";
  return (
    <li
      className="track-row"
      style={{ transform: `translateY(${top}px)` }}
      title={track.path}
    >
      <Cover track={track} />
      <span className="track-main">
        <span className="track-title">{track.title}</span>
        <span className="track-sub">
          {artist}
          {album ? ` · ${album}` : ""}
        </span>
      </span>
      <span className="track-meta">
        <span className="track-badge">{formatBadge(track.format)}</span>
        <span className="track-dur">{formatDuration(track.durationMs)}</span>
      </span>
    </li>
  );
}

/** Fixed-row-height virtualized list (hand-rolled — no dependency, React-19-safe). */
function TrackList({ tracks }: { tracks: Track[] }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewH, setViewH] = useState(0);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewH(el.clientHeight));
    ro.observe(el);
    setViewH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  const onScroll = useCallback(() => {
    setScrollTop(scrollRef.current?.scrollTop ?? 0);
  }, []);

  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 8);
  const end = Math.min(tracks.length, Math.ceil((scrollTop + viewH) / ROW_HEIGHT) + 8);
  const rows: ReactNode[] = [];
  for (let i = start; i < end; i++) {
    const t = tracks[i];
    rows.push(<TrackRow key={t.id === 0 ? t.path : t.id} track={t} top={i * ROW_HEIGHT} />);
  }

  return (
    <div className="track-scroll" ref={scrollRef} onScroll={onScroll}>
      <ul className="track-inner" style={{ height: tracks.length * ROW_HEIGHT }}>
        {rows}
      </ul>
    </div>
  );
}

function Skeleton({ count }: { count: number }) {
  return (
    <div className="skeleton" aria-hidden="true">
      {Array.from({ length: Math.min(count, 14) }, (_, i) => (
        <div key={i} className="sk-row" style={{ top: i * ROW_HEIGHT }} />
      ))}
    </div>
  );
}

function Shell({ children }: { children: ReactNode }) {
  return (
    <main className="app">
      <aside className="sidebar">
        <div className="brand-row">
          <img className="brand-mark" src={iwaksMark} alt="" />
          <span className="brand-name">Iwaks</span>
        </div>
        <nav className="nav" aria-label="Library">
          <a className="nav-item active" href="#songs" aria-current="page">
            Songs
          </a>
          <a className="nav-item" href="#albums">
            Albums
          </a>
          <a className="nav-item" href="#artists">
            Artists
          </a>
          <a className="nav-item" href="#playlists">
            Playlists
          </a>
          <a className="nav-item" href="#folders">
            Folders
          </a>
        </nav>
      </aside>
      <section className="main" id="songs">
        {children}
      </section>
    </main>
  );
}

function App() {
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanProg, setScanProg] = useState<ScanProgress | null>(null);
  const [scanNote, setScanNote] = useState<string | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const tracks = useSearch(query);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listenScan({
      onProgress: (p, finished) => {
        if (disposed) return;
        setScanProg(p);
        if (finished) {
          setScanning(false);
          setScanNote(scanSummary(p));
        }
      },
      onError: (msg) => {
        if (disposed) return;
        setScanning(false);
        setBanner(`Scan failed — ${msg}`);
      },
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Track-level load states: the search hook owns data; a failed load shows a banner.
  useEffect(() => {
    if (tracks === null && status === "loading") {
      const t = setTimeout(
        () => getTracks().catch((e) => setLoadError(String(e))).finally(() => setStatus("ready")),
        1200,
      );
      return () => clearTimeout(t);
    }
    if (tracks !== null && status === "loading") setStatus("ready");
  }, [tracks, status]);

  const onScan = useCallback(async () => {
    setBanner(null);
    setScanNote(null);
    const folder = await pickFolder();
    if (!folder) return;
    try {
      setScanning(true);
      setScanProg({ totalFiles: 0, scanned: 0, added: 0, updated: 0, skipped: 0, removed: 0, errors: 0 });
      await scanFolder(folder);
    } catch (e) {
      setScanning(false);
      setBanner(`Couldn't start a scan — ${String(e)}`);
    }
  }, []);

  const retryLoad = useCallback(() => {
    setStatus("loading");
    setLoadError(null);
    setQuery("");
  }, []);

  const showing = tracks ?? [];
  const isEmpty = status === "ready" && tracks !== null && showing.length === 0;
  const progressPct =
    scanProg && scanProg.totalFiles > 0
      ? Math.min(100, Math.round((scanProg.scanned / scanProg.totalFiles) * 100))
      : 0;

  return (
    <Shell>
      <div className="toolbar">
        <div className="toolbar-title">
          <h1>Songs</h1>
          {tracks !== null && !isEmpty && (
            <span className="count">{showing.length.toLocaleString()}</span>
          )}
        </div>
        <div className="toolbar-actions">
          <label className="search">
            <span className="visually-hidden">Search your library</span>
            <input
              type="search"
              placeholder="Search songs, artists, albums…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          <button className="btn-primary" onClick={onScan} disabled={scanning}>
            {scanning ? "Scanning…" : "Add folder"}
          </button>
        </div>
      </div>

      {scanning && scanProg && (
        <div className="progress" role="status" aria-label="Scanning your music folder">
          <div className="progress-bar">
            <div className="progress-fill" style={{ width: `${progressPct}%` }} />
          </div>
          <span className="progress-label">
            Scanning… {scanProg.scanned.toLocaleString()}
            {scanProg.totalFiles > 0
              ? ` of ${scanProg.totalFiles.toLocaleString()} files`
              : " files"}
          </span>
        </div>
      )}

      {scanNote && (
        <p className="scan-note">
          Scan finished — {scanNote}
          {query.trim() !== "" && " · showing results for “" + query.trim() + "”"}
        </p>
      )}
      {banner && (
        <div className="banner" role="alert">
          <span>{banner}</span>
          <button className="btn-ghost" onClick={() => setBanner(null)}>
            Dismiss
          </button>
        </div>
      )}

      <div className="track-head" aria-hidden="true">
        <span>Title</span>
        <span className="track-head-dur">Time</span>
      </div>

      <div className="list-wrap">
        {status === "loading" && tracks === null ? (
          <Skeleton count={16} />
        ) : isEmpty ? (
          <div className="empty">
            <img className="empty-mark" src={iwaksMark} alt="" />
            <h2>{scanning ? "Building your library…" : "Your music lives here"}</h2>
            <p>
              {scanning
                ? "Keep this window open while Iwaks finds your tracks."
                : "Point Iwaks at a folder on this PC and it will scan hi-res & lossless formats."}
            </p>
            <button className="btn-primary" onClick={onScan} disabled={scanning}>
              {scanning ? "Scanning…" : "Scan a folder"}
            </button>
          </div>
        ) : loadError || status === "error" ? (
          <div className="error-state">
            <h2>Couldn't load your library</h2>
            <p>{loadError ?? "Something went wrong while reading the database."}</p>
            <button className="btn-primary" onClick={retryLoad}>
              Try again
            </button>
          </div>
        ) : (
          <TrackList tracks={showing} />
        )}
      </div>
    </Shell>
  );
}

export default App;