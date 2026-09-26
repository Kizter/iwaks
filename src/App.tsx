import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  addFiles,
  addToPlaylist,
  createPlaylist,
  deletePlaylist,
  exportM3u,
  getPlayerState,
  getPlaylistTracks,
  getTracks,
  importM3u,
  listenPlayer,
  listenScan,
  listPlaylists,
  pickFiles,
  pickFolder,
  pickM3uFile,
  pickM3uSave,
  playTracks,
  playerTogglePlay,
  removeFromPlaylist,
  renamePlaylist,
  scanFolder,
  searchTracks,
} from "./api";
import "./App.css";
import { formatBadge, formatDuration, scanSummary } from "./format";
import { Cover } from "./Cover";
import PlayerBar from "./PlayerBar";
import type { Playlist, PlayerState, ScanProgress, Track } from "./types";
import iwaksMark from "./assets/iwaks-mark.png";

const ROW_HEIGHT = 56;

// ---- library navigation views (grouped browse) ----

type View =
  | { kind: "songs" }
  | { kind: "albums" }
  | { kind: "artists" }
  | { kind: "folders" }
  | { kind: "playlists" }
  | { kind: "playlist"; id: number }
  | { kind: "album"; key: string }
  | { kind: "artist"; key: string }
  | { kind: "folder"; key: string };

type GroupKind = "album" | "artist" | "folder";

/** Directory of a track file — the folder-grouping key. */
function dirOf(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i < 0 ? path : path.slice(0, i);
}

/** Last path segment of a directory (the display name of a folder group). */
function nameOfDir(dir: string): string {
  const trimmed = dir.replace(/[\\/]$/, "");
  const i = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return i < 0 ? trimmed : trimmed.slice(i + 1);
}

/** Group key a track belongs to — must match `groupBy` exactly. */
function groupKeyOf(track: Track, kind: GroupKind): string {
  if (kind === "album") return track.album ?? "(Unknown album)";
  if (kind === "artist") return track.artist ?? "Unknown artist";
  return dirOf(track.path);
}

/** Case-insensitive title/artist/album filter (playlist detail search). */
function matches(track: Track, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return (
    track.title.toLowerCase().includes(q) ||
    (track.artist ?? "").toLowerCase().includes(q) ||
    (track.album ?? "").toLowerCase().includes(q)
  );
}

interface Group {
  key: string;
  label: string;
  sub: string;
  tracks: Track[];
}

/** Group tracks by album / artist / containing folder (case-insensitive alpha). */
function groupBy(tracks: Track[], kind: GroupKind): Group[] {
  const index = new Map<string, Track[]>();
  for (const t of tracks) {
    const key = groupKeyOf(t, kind);
    const list = index.get(key);
    if (list) list.push(t);
    else index.set(key, [t]);
  }
  const groups: Group[] = [];
  for (const [key, list] of index) {
    const count = `${list.length} ${list.length === 1 ? "track" : "tracks"}`;
    groups.push({
      key,
      label: kind === "folder" ? nameOfDir(key) : key,
      sub: kind === "album" ? (list[0].albumArtist ?? list[0].artist ?? "Unknown artist") : count,
      tracks: list,
    });
  }
  return groups.sort((a, b) => a.label.localeCompare(b.label, undefined, { sensitivity: "base" }));
}

/** Debounced search-as-you-type hook. Blank query → full library. */
function useSearch(query: string, refreshKey: number) {
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
    // refreshKey: bump after scan/add so the list reflects new rows.
  }, [query, refreshKey]);
  return tracks;
}

function PlusIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path fill="currentColor" d="M7 2h2v5h5v2H9v5H7V9H2V7h5z" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true">
      <path
        fill="currentColor"
        d="m4.6 3.5 3.4 3.4 3.4-3.4 1.1 1.1-3.4 3.4 3.4 3.4-1.1 1.1-3.4-3.4-3.4 3.4-1.1-1.1 3.4-3.4-3.4-3.4z"
      />
    </svg>
  );
}

function TrackRow({
  track,
  top,
  isPlaying,
  onPlay,
  onAdd,
  onRemove,
}: {
  track: Track;
  top: number;
  isPlaying: boolean;
  onPlay: () => void;
  onAdd?: (t: Track) => void;
  onRemove?: (t: Track) => void;
}) {
  const artist = track.artist ?? "Unknown artist";
  const album = track.album ?? "";
  return (
    <li
      className={`track-row${isPlaying ? " is-playing" : ""}`}
      style={{ transform: `translateY(${top}px)` }}
      title={track.path}
      onClick={onPlay}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onPlay();
        }
      }}
      tabIndex={0}
      role="button"
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
        {onAdd && (
          <button
            type="button"
            className="row-btn"
            title="Add to playlist"
            aria-label={`Add ${track.title} to a playlist`}
            onClick={(e) => {
              e.stopPropagation();
              onAdd(track);
            }}
          >
            <PlusIcon />
          </button>
        )}
        {onRemove && (
          <button
            type="button"
            className="row-btn"
            title="Remove from playlist"
            aria-label={`Remove ${track.title} from this playlist`}
            onClick={(e) => {
              e.stopPropagation();
              onRemove(track);
            }}
          >
            <CloseIcon />
          </button>
        )}
        <span className="track-badge">{formatBadge(track.format)}</span>
        <span className="track-dur">{formatDuration(track.durationMs)}</span>
      </span>
    </li>
  );
}

/** Back arrow for toolbar title when browsing inside a group. */
function BackIcon() {
  return (
    <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
      <path fill="currentColor" d="M9.8 3.4 5.2 8l4.6 4.6-1.1 1.1L3 8l5.7-5.7z" />
    </svg>
  );
}

/** Fixed-row-height virtualized list (hand-rolled — no dependency, React-19-safe). */
function TrackList({
  tracks,
  playingPath,
  onPlay,
  onAdd,
  onRemove,
}: {
  tracks: Track[];
  playingPath: string | null;
  onPlay: (index: number) => void;
  onAdd?: (t: Track) => void;
  onRemove?: (t: Track) => void;
}) {
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
    rows.push(
      <TrackRow
        key={t.id === 0 ? t.path : t.id}
        track={t}
        top={i * ROW_HEIGHT}
        isPlaying={t.path === playingPath}
        onPlay={() => onPlay(i)}
        onAdd={onAdd}
        onRemove={onRemove}
      />,
    );
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

const NAV_ITEMS: Array<{
  id: "songs" | "albums" | "artists" | "folders" | "playlists";
  label: string;
}> = [
  { id: "songs", label: "Songs" },
  { id: "albums", label: "Albums" },
  { id: "artists", label: "Artists" },
  { id: "folders", label: "Folders" },
  { id: "playlists", label: "Playlists" },
];

/** Which sidebar entries the current view belongs under (grid or its detail). */
function homeOf(view: View): View["kind"] {
  if (view.kind === "albums" || view.kind === "album") return "albums";
  if (view.kind === "artists" || view.kind === "artist") return "artists";
  if (view.kind === "folders" || view.kind === "folder") return "folders";
  if (view.kind === "playlists" || view.kind === "playlist") return "playlists";
  return "songs";
}

/** Small inline input for naming/renaming a playlist (toolbar). */
function InlineName({
  initial,
  placeholder,
  onSave,
  onCancel,
}: {
  initial: string;
  placeholder?: string;
  onSave: (name: string) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  return (
    <form
      className="inline-name"
      onSubmit={(e) => {
        e.preventDefault();
        if (draft.trim()) onSave(draft.trim());
      }}
    >
      <input
        autoFocus
        value={draft}
        placeholder={placeholder}
        aria-label="Playlist name"
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
        }}
      />
      <button type="submit" className="btn-ghost" disabled={!draft.trim()}>
        Save
      </button>
      <button type="button" className="btn-ghost" onClick={onCancel}>
        Cancel
      </button>
    </form>
  );
}

function Shell({
  children,
  playerState,
  view,
  onNavigate,
}: {
  children: ReactNode;
  playerState: PlayerState | null;
  view: View;
  onNavigate: (v: View) => void;
}) {
  const home = homeOf(view);
  return (
    <main className="app">
      <aside className="sidebar">
        <div className="brand-row">
          <img className="brand-mark" src={iwaksMark} alt="" />
          <span className="brand-name">Iwaks</span>
        </div>
        <nav className="nav" aria-label="Library">
          {NAV_ITEMS.map((item) => {
            const active = home === item.id;
            return (
              <button
                key={item.id}
                type="button"
                className={`nav-item${active ? " active" : ""}`}
                aria-current={active ? "page" : undefined}
                onClick={() => onNavigate({ kind: item.id })}
              >
                {item.label}
              </button>
            );
          })}
        </nav>
      </aside>
      <section className="main">{children}</section>
      <PlayerBar state={playerState} />
    </main>
  );
}

function App() {
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<"loading" | "ready" | "error">("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [adding, setAdding] = useState(false);
  const [scanProg, setScanProg] = useState<ScanProgress | null>(null);
  const [scanNote, setScanNote] = useState<string | null>(null);
  const [banner, setBanner] = useState<string | null>(null);
  const [playerState, setPlayerState] = useState<PlayerState | null>(null);
  const [playerError, setPlayerError] = useState<string | null>(null);
  // Bumped after a scan/add finishes so the track list re-fetches.
  const [refreshKey, setRefreshKey] = useState(0);
  // Active library view: song list, browse grid, or a specific group detail.
  const [view, setView] = useState<View>({ kind: "songs" });
  const tracks = useSearch(query, refreshKey);

  // ---- playlists (M4 slice 1) ----
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);
  const [plDetail, setPlDetail] = useState<{ name: string; tracks: Track[] } | null>(null);
  // Track awaiting a playlist choice (the "+" button on a row).
  const [addTarget, setAddTarget] = useState<Track | null>(null);
  const [plDraft, setPlDraft] = useState("");
  const [creating, setCreating] = useState(false);
  const [renaming, setRenaming] = useState(false);

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
          setRefreshKey((k) => k + 1); // bring the new rows into the list
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

  // Playback: initial state + live updates / init errors.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getPlayerState()
      .then((s) => {
        if (!disposed) setPlayerState(s);
      })
      .catch(() => {});
    listenPlayer(
      (s) => {
        if (disposed) return;
        setPlayerState(s);
        setPlayerError(null);
      },
      (msg) => {
        if (disposed) return;
        setPlayerError(msg);
      },
    ).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Space toggles play/pause (never while typing or when a button is focused).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      if (!el) return;
      const tag = el.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "BUTTON" ||
        el.isContentEditable ||
        el.closest("[role='button']")
      ) {
        return;
      }
      if (e.code === "Space") {
        e.preventDefault();
        void playerTogglePlay();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
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

  // ---- playlists: list load, detail load on entry, picker Esc handler ----
  useEffect(() => {
    let alive = true;
    listPlaylists()
      .then((l) => {
        if (alive) setPlaylists(l);
      })
      .catch((e) => {
        if (alive) setBanner(`Couldn't load playlists — ${String(e)}`);
      });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (view.kind !== "playlist") return;
    let alive = true;
    setPlDetail(null);
    setRenaming(false);
    (async () => {
      try {
        const detail = await getPlaylistTracks(view.id);
        if (!alive) return;
        if (detail === null) {
          setView({ kind: "playlists" });
          return;
        }
        const list = await listPlaylists();
        if (!alive) return;
        setPlaylists(list);
        setPlDetail({
          name: list.find((p) => p.id === view.id)?.name ?? "Playlist",
          tracks: detail,
        });
      } catch (e) {
        if (alive) setBanner(`Couldn't load playlist — ${String(e)}`);
      }
    })();
    return () => {
      alive = false;
    };
  }, [view]);

  useEffect(() => {
    if (!addTarget) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setAddTarget(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [addTarget]);

  const refreshPlaylists = useCallback(async () => {
    try {
      const list = await listPlaylists();
      setPlaylists(list);
      if (view.kind === "playlist") {
        const current = list.find((p) => p.id === view.id);
        if (!current) {
          setView({ kind: "playlists" });
          return;
        }
        const detail = await getPlaylistTracks(current.id);
        setPlDetail({ name: current.name, tracks: detail ?? [] });
      }
    } catch (e) {
      setBanner(`Playlist update failed — ${String(e)}`);
    }
  }, [view]);

  const createNew = useCallback(
    async (name: string) => {
      try {
        const id = await createPlaylist(name);
        setCreating(false);
        await refreshPlaylists();
        setView({ kind: "playlist", id });
      } catch (e) {
        setBanner(`Couldn't create playlist — ${String(e)}`);
      }
    },
    [refreshPlaylists],
  );

  const onRename = useCallback(
    async (name: string) => {
      if (view.kind !== "playlist") return;
      try {
        await renamePlaylist(view.id, name);
        setRenaming(false);
        await refreshPlaylists();
      } catch (e) {
        setBanner(`Rename failed — ${String(e)}`);
      }
    },
    [view, refreshPlaylists],
  );

  const onImportM3u = useCallback(async () => {
    const path = await pickM3uFile();
    if (!path) return;
    try {
      const id = await importM3u(path);
      setBanner("Playlist imported");
      await refreshPlaylists();
      setView({ kind: "playlist", id });
    } catch (e) {
      setBanner(`Import failed — ${String(e)}`);
    }
  }, [refreshPlaylists]);

  const onExport = useCallback(async () => {
    if (view.kind !== "playlist") return;
    const current = (playlists ?? []).find((p) => p.id === view.id);
    const picked = await pickM3uSave(`${current?.name ?? "playlist"}.m3u`);
    if (!picked) return;
    try {
      await exportM3u(view.id, picked);
      setBanner("Playlist exported");
    } catch (e) {
      setBanner(`Export failed — ${String(e)}`);
    }
  }, [view, playlists]);

  const onDelete = useCallback(async () => {
    if (view.kind !== "playlist") return;
    const current = (playlists ?? []).find((p) => p.id === view.id);
    const ok = window.confirm(`Delete playlist “${current?.name ?? "this playlist"}”?`);
    if (!ok) return;
    try {
      await deletePlaylist(view.id);
      setView({ kind: "playlists" });
      await refreshPlaylists();
    } catch (e) {
      setBanner(`Delete failed — ${String(e)}`);
    }
  }, [view, playlists, refreshPlaylists]);

  const onRemoveFromDetail = useCallback(
    async (track: Track) => {
      if (view.kind !== "playlist") return;
      try {
        await removeFromPlaylist(view.id, track.id);
        await refreshPlaylists();
      } catch (e) {
        setBanner(`Couldn't remove track — ${String(e)}`);
      }
    },
    [view, refreshPlaylists],
  );

  const addTrackToPlaylist = useCallback(
    async (playlistId: number) => {
      const t = addTarget;
      if (!t) return;
      try {
        await addToPlaylist(playlistId, t.id);
        setBanner(`Added “${t.title}” to playlist`);
        setAddTarget(null);
        await refreshPlaylists();
      } catch (e) {
        setBanner(`Couldn't add track — ${String(e)}`);
      }
    },
    [addTarget, refreshPlaylists],
  );

  const addTrackNewPlaylist = useCallback(
    async (name: string) => {
      const t = addTarget;
      if (!t) return;
      try {
        const id = await createPlaylist(name);
        await addToPlaylist(id, t.id);
        setBanner(`Created playlist with “${t.title}”`);
        setAddTarget(null);
        await refreshPlaylists();
        setView({ kind: "playlist", id });
      } catch (e) {
        setBanner(`Couldn't add track — ${String(e)}`);
      }
    },
    [addTarget, refreshPlaylists],
  );

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

  const onAddFiles = useCallback(async () => {
    setBanner(null);
    setScanNote(null);
    const files = await pickFiles();
    if (!files || files.length === 0) return;
    try {
      setAdding(true);
      const report = await addFiles(files);
      const noun = report.added === 1 ? "file" : "files";
      setScanNote(
        `Added ${report.added.toLocaleString()} ${noun}${report.errors > 0 ? ` · ${report.errors} skipped` : ""}`,
      );
      setRefreshKey((k) => k + 1); // bring the new rows into the list
    } catch (e) {
      setBanner(`Couldn't add files — ${String(e)}`);
    } finally {
      setAdding(false);
    }
  }, []);

  const retryLoad = useCallback(() => {
    setStatus("loading");
    setLoadError(null);
    setQuery("");
  }, []);

  const showing = tracks ?? [];
  const progressPct =
    scanProg && scanProg.totalFiles > 0
      ? Math.min(100, Math.round((scanProg.scanned / scanProg.totalFiles) * 100))
      : 0;

  // ---- browse views: group grid / group detail, derived from the library ----
  const gridKind =
    view.kind === "albums" || view.kind === "artists" || view.kind === "folders"
      ? view.kind
      : null;
  const groupKind: GroupKind | null =
    view.kind === "album" || view.kind === "artist" || view.kind === "folder" ? view.kind : null;
  const groupKey: string | null = groupKind ? (view as { key: string }).key : null;
  const groups = useMemo(
    () =>
      gridKind
        ? groupBy(showing, gridKind === "albums" ? "album" : gridKind === "artists" ? "artist" : "folder")
        : [],
    [gridKind, showing],
  );
  const groupTracks = useMemo(
    () =>
      groupKind && groupKey ? showing.filter((t) => groupKeyOf(t, groupKind) === groupKey) : null,
    [groupKind, groupKey, showing],
  );

  const isGrid = gridKind !== null;
  const isPlaylists = view.kind === "playlists";
  const isPlaylistDetail = view.kind === "playlist";
  const currentPlaylist = isPlaylistDetail
    ? (playlists ?? []).find((p) => p.id === (view as { id: number }).id) ?? null
    : null;
  const plName = currentPlaylist?.name ?? plDetail?.name ?? "Playlist";
  const filteredPlTracks =
    isPlaylistDetail && plDetail ? plDetail.tracks.filter((t) => matches(t, query)) : null;
  const ready = status === "ready" && tracks !== null;
  const isEmpty = ready
    ? isPlaylists
      ? playlists !== null && playlists.length === 0
      : isPlaylistDetail
        ? plDetail !== null && plDetail.tracks.length === 0 && query.trim() === ""
        : isGrid
          ? groups.length === 0
          : showing.length === 0
    : false;
  const activeTracks = filteredPlTracks ?? groupTracks ?? showing;

  const onPlay = useCallback(
    (index: number) => {
      if (index < 0 || index >= activeTracks.length) return;
      void playTracks(activeTracks, index);
    },
    [activeTracks],
  );

  const detailTitle =
    groupKind === "folder" && groupKey ? nameOfDir(groupKey) : (groupKey ?? "");
  const pageTitle =
    view.kind === "songs"
      ? "Songs"
      : view.kind === "albums"
        ? "Albums"
        : view.kind === "artists"
          ? "Artists"
          : view.kind === "folders"
            ? "Folders"
            : view.kind === "playlists"
              ? "Playlists"
              : view.kind === "playlist"
                ? plName
                : detailTitle;
  const count = isGrid
    ? groups.length
    : isPlaylists
      ? playlists?.length ?? 0
      : isPlaylistDetail
        ? filteredPlTracks?.length ?? 0
        : groupKind && groupTracks
          ? groupTracks.length
          : showing.length;

  return (
    <Shell playerState={playerState} view={view} onNavigate={setView}>
      <div className="toolbar">
        <div className="toolbar-title">
          {(groupKind || isPlaylistDetail) && (
            <button
              type="button"
              className="back-btn"
              aria-label={
                isPlaylistDetail
                  ? "Back to Playlists"
                  : `Back to ${groupKind === "album" ? "Albums" : groupKind === "artist" ? "Artists" : "Folders"}`
              }
              title="Back"
              onClick={() =>
                isPlaylistDetail
                  ? setView({ kind: "playlists" })
                  : setView({ kind: groupKind === "album" ? "albums" : groupKind === "artist" ? "artists" : "folders" })
              }
            >
              <BackIcon />
            </button>
          )}
          <h1>{pageTitle}</h1>
          {(tracks !== null || isPlaylists || isPlaylistDetail) && !isEmpty && (
            <span className="count">{count.toLocaleString()}</span>
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
          <button
            className="btn-ghost"
            onClick={() => void onAddFiles()}
            disabled={scanning || adding}
          >
            {adding ? "Adding…" : "Add files"}
          </button>
          <button className="btn-primary" onClick={onScan} disabled={scanning || adding}>
            {scanning ? "Scanning…" : "Add folder"}
          </button>
          {isPlaylists && (
            <>
              {creating ? (
                <InlineName
                  initial=""
                  placeholder="Playlist name…"
                  onSave={(n) => void createNew(n)}
                  onCancel={() => setCreating(false)}
                />
              ) : (
                <button
                  className="btn-ghost"
                  onClick={() => setCreating(true)}
                  disabled={scanning || adding}
                >
                  New playlist
                </button>
              )}
              <button
                className="btn-ghost"
                onClick={() => void onImportM3u()}
                disabled={scanning || adding}
              >
                Import .m3u
              </button>
            </>
          )}
          {isPlaylistDetail && currentPlaylist && (
            <>
              {renaming ? (
                <InlineName
                  initial={currentPlaylist.name}
                  onSave={(n) => void onRename(n)}
                  onCancel={() => setRenaming(false)}
                />
              ) : (
                <button
                  className="btn-ghost"
                  onClick={() => setRenaming(true)}
                  disabled={scanning || adding}
                >
                  Rename
                </button>
              )}
              <button className="btn-ghost" onClick={() => void onExport()}>
                Export
              </button>
              <button className="btn-danger" onClick={() => void onDelete()}>
                Delete
              </button>
            </>
          )}
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
      {playerError && (
        <div className="banner" role="alert">
          <span>{playerError}</span>
          <button className="btn-ghost" onClick={() => setPlayerError(null)}>
            Dismiss
          </button>
        </div>
      )}
      {banner && (
        <div className="banner" role="alert">
          <span>{banner}</span>
          <button className="btn-ghost" onClick={() => setBanner(null)}>
            Dismiss
          </button>
        </div>
      )}

      {!isGrid && !isPlaylists && (
        <div className="track-head" aria-hidden="true">
          <span>Title</span>
          <span className="track-head-dur">Time</span>
        </div>
      )}

      <div className="list-wrap">
        {status === "loading" && tracks === null ? (
          <Skeleton count={16} />
        ) : (isPlaylists && playlists === null) || (isPlaylistDetail && plDetail === null) ? (
          <Skeleton count={12} />
        ) : isEmpty && isPlaylists ? (
          <div className="empty">
            <img className="empty-mark" src={iwaksMark} alt="" />
            <h2>No playlists yet</h2>
            <p>Create a playlist, or import an .m3u file from disk.</p>
            <div className="empty-actions">
              <button className="btn-primary" onClick={() => setCreating(true)}>
                New playlist
              </button>
              <button className="btn-ghost" onClick={() => void onImportM3u()}>
                Import .m3u
              </button>
            </div>
          </div>
        ) : isEmpty && isPlaylistDetail ? (
          <div className="empty">
            <h2>This playlist is empty</h2>
            <p>Use the “+” button next to any song to add it here.</p>
          </div>
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
        ) : isPlaylists ? (
          <div className="group-grid">
            {(playlists ?? []).map((p) => (
              <button
                key={p.id}
                type="button"
                className="group-card playlist-card"
                onClick={() => setView({ kind: "playlist", id: p.id })}
              >
                <span className="playlist-art" aria-hidden="true">
                  <svg viewBox="0 0 24 24" width="30" height="30">
                    <rect
                      x="3"
                      y="4"
                      width="18"
                      height="16"
                      rx="3"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.8"
                    />
                    <path
                      d="M8 9h8M8 13h8M8 17h5"
                      stroke="currentColor"
                      strokeWidth="1.8"
                      strokeLinecap="round"
                    />
                  </svg>
                </span>
                <span className="group-name">{p.name}</span>
                <span className="group-sub">
                  {p.trackCount.toLocaleString()} {p.trackCount === 1 ? "song" : "songs"}
                </span>
              </button>
            ))}
          </div>
        ) : isPlaylistDetail && filteredPlTracks && filteredPlTracks.length === 0 ? (
          <p className="scan-note">No tracks in this playlist match your search.</p>
        ) : isPlaylistDetail && filteredPlTracks ? (
          <TrackList
            tracks={filteredPlTracks}
            playingPath={playerState?.current?.path ?? null}
            onPlay={onPlay}
            onRemove={(t) => void onRemoveFromDetail(t)}
          />
        ) : gridKind ? (
          <div className="group-grid">
            {groups.map((g) => (
              <button
                key={`${gridKind}-${g.key}`}
                type="button"
                className="group-card"
                title={g.key}
                onClick={() =>
                  setView(
                    gridKind === "albums"
                      ? { kind: "album", key: g.key }
                      : gridKind === "artists"
                        ? { kind: "artist", key: g.key }
                        : { kind: "folder", key: g.key },
                  )
                }
              >
                <Cover track={g.tracks[0]} />
                <span className="group-name">{g.label}</span>
                <span className="group-sub">
                  {g.sub}
                  {" · "}
                  {g.tracks.length.toLocaleString()} {g.tracks.length === 1 ? "song" : "songs"}
                </span>
              </button>
            ))}
          </div>
        ) : groupKind && groupTracks && groupTracks.length === 0 ? (
          <p className="scan-note">No tracks in this {groupKind} match your search.</p>
        ) : (
          <TrackList
            tracks={groupTracks ?? showing}
            playingPath={playerState?.current?.path ?? null}
            onPlay={onPlay}
            onAdd={(t) => setAddTarget(t)}
          />
        )}
      </div>

      {addTarget && (
        <div className="picker-backdrop" onClick={() => setAddTarget(null)}>
          <div
            className="picker"
            role="dialog"
            aria-modal="true"
            aria-label="Add to playlist"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="picker-title">
              Add “{addTarget.title}” to…
            </p>
            <div className="picker-list">
              {(playlists ?? []).map((p) => (
                <button
                  key={p.id}
                  type="button"
                  className="picker-item"
                  onClick={() => void addTrackToPlaylist(p.id)}
                >
                  <span>{p.name}</span>
                  <span className="count">{p.trackCount}</span>
                </button>
              ))}
              {(playlists?.length ?? 0) === 0 && (
                <p className="picker-empty">No playlists yet — create one below.</p>
              )}
            </div>
            <form
              className="picker-new"
              onSubmit={(e) => {
                e.preventDefault();
                if (plDraft.trim()) void addTrackNewPlaylist(plDraft.trim());
              }}
            >
              <input
                autoFocus
                value={plDraft}
                placeholder="New playlist name…"
                aria-label="New playlist name"
                onChange={(e) => setPlDraft(e.target.value)}
              />
              <button type="submit" className="btn-primary" disabled={!plDraft.trim()}>
                Create &amp; add
              </button>
            </form>
          </div>
        </div>
      )}
    </Shell>
  );
}

export default App;