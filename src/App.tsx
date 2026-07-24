import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Check,
  ChevronLeft,
  ChevronRight,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  Gauge,
  Gamepad2,
  HardDrive,
  Library,
  Link2,
  LoaderCircle,
  Moon,
  PackageCheck,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Sun,
  Trash2,
  X
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import "./App.css";
import {
  API_ORIGIN,
  buildChartCoverUrl,
  buildChartPreviewUrl,
  buildChartPublicUrl,
  chartContentVersion,
  difficultyClass,
  formatBytes,
  formatDuration,
  parseInstallDeepLink,
  type AppState,
  type Chart,
  type ChartCatalog,
  type InstallProgress,
  type InstallResult,
  type InstalledChart,
  type ManualChartMatch,
  type TrashItem,
  type UpdateCandidate
} from "./lib";

type View = "charts" | "downloads" | "settings";
type Theme = "light" | "dark";
type InstallEntry = {
  chart: Chart;
  error?: string;
  installPath?: string;
  progress: InstallProgress;
};

const difficulties = ["", "beginner", "normal", "hard", "expert", "UNBEATABLE", "STAR"];
const isTauri = () => "__TAURI_INTERNALS__" in window;

async function openExternalUrl(url: string) {
  if (isTauri()) {
    await invoke("open_external_url", { url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

async function fetchCatalog(query: string, page: number, difficulty: string, rankedOnly: boolean) {
  if (isTauri()) {
    return invoke<ChartCatalog>("fetch_charts", { difficulty, page, query, rankedOnly });
  }
  const params = new URLSearchParams({ page: String(page), pageSize: "24", sort: "newest" });
  if (query) params.set("q", query);
  if (difficulty) params.set("difficulty", difficulty);
  if (rankedOnly) params.set("ranked", "1");
  const response = await fetch(`${API_ORIGIN}/api/charts?${params}`);
  if (!response.ok) throw new Error("Could not load charts.");
  return response.json() as Promise<ChartCatalog>;
}

async function fetchChart(chartId: string) {
  if (isTauri()) return invoke<Chart>("fetch_chart", { chartId });
  const response = await fetch(`${API_ORIGIN}/api/charts/${encodeURIComponent(chartId)}`);
  const payload = await response.json() as { chart?: Chart };
  if (!response.ok || !payload.chart) throw new Error("Could not load this chart.");
  return payload.chart;
}

function App() {
  const [view, setView] = useState<View>("charts");
  const [theme, setTheme] = useState<Theme>(() => {
    const savedTheme = localStorage.getItem("unchartable:theme");
    if (savedTheme === "light" || savedTheme === "dark") return savedTheme;
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  });
  const [appState, setAppState] = useState<AppState | null>(null);
  const [targetDirectory, setTargetDirectory] = useState(() => localStorage.getItem("unchartable:custom-songs") || "");
  const [queryInput, setQueryInput] = useState("");
  const [query, setQuery] = useState("");
  const [difficulty, setDifficulty] = useState("");
  const [rankedOnly, setRankedOnly] = useState(false);
  const [page, setPage] = useState(0);
  const [catalog, setCatalog] = useState<ChartCatalog>({ charts: [], count: 0, nextPage: null });
  const [loading, setLoading] = useState(true);
  const [catalogError, setCatalogError] = useState("");
  const [installing, setInstalling] = useState<Record<string, InstallEntry>>({});
  const [pendingInstallId, setPendingInstallId] = useState<string | null>(null);
  const [installedCharts, setInstalledCharts] = useState<InstalledChart[]>([]);
  const [installedQuery, setInstalledQuery] = useState("");
  const [manualMatches, setManualMatches] = useState<ManualChartMatch[]>([]);
  const [trashItems, setTrashItems] = useState<TrashItem[]>([]);
  const [libraryLoading, setLibraryLoading] = useState(false);
  const [updateMessage, setUpdateMessage] = useState("");
  const [previewingId, setPreviewingId] = useState<string | null>(null);
  const [previewErrorId, setPreviewErrorId] = useState<string | null>(null);
  const previewAudioRef = useRef<HTMLAudioElement | null>(null);
  const [automaticUpdates, setAutomaticUpdates] = useState(
    () => localStorage.getItem("unchartable:auto-updates") !== "off"
  );

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("unchartable:theme", theme);
  }, [theme]);

  useEffect(() => () => {
    previewAudioRef.current?.pause();
    previewAudioRef.current = null;
  }, []);

  useEffect(() => {
    if (!isTauri()) return;

    const preventContextMenu = (event: MouseEvent) => event.preventDefault();
    const preventBrowserShortcuts = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      const browserCommand =
        ((event.ctrlKey || event.metaKey) && ["p", "r", "s", "u"].includes(key)) ||
        event.key === "F5" ||
        (event.altKey && (event.key === "ArrowLeft" || event.key === "ArrowRight"));
      if (browserCommand) event.preventDefault();
    };

    document.addEventListener("contextmenu", preventContextMenu);
    document.addEventListener("keydown", preventBrowserShortcuts);
    return () => {
      document.removeEventListener("contextmenu", preventContextMenu);
      document.removeEventListener("keydown", preventBrowserShortcuts);
    };
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    const acceptDeepLinks = (urls: string[]) => {
      const chartId = urls.map(parseInstallDeepLink).find((value): value is string => Boolean(value));
      if (chartId) setPendingInstallId(chartId);
    };

    void onOpenUrl(acceptDeepLinks).then((cleanup) => {
      unlisten = cleanup;
    });
    void getCurrent().then((urls) => {
      if (urls) acceptDeepLinks(urls);
    });

    return () => unlisten?.();
  }, []);

  useEffect(() => {
    const timeout = window.setTimeout(() => {
      setPage(0);
      setQuery(queryInput.trim());
    }, 280);
    return () => window.clearTimeout(timeout);
  }, [queryInput]);

  useEffect(() => {
    if (!isTauri()) {
      const fallback = `${navigator.userAgent.includes("Windows") ? "C:\\Users\\You\\AppData\\LocalLow" : ""}\\D-CELL GAMES\\UNBEATABLE\\CustomSongs`;
      setAppState({ customSongsPath: fallback, directoryExists: false });
      if (!targetDirectory) setTargetDirectory(fallback);
      return;
    }
    const savedDirectory = localStorage.getItem("unchartable:custom-songs");
    const stateRequest = savedDirectory
      ? invoke<AppState>("validate_custom_songs_path", { path: savedDirectory }).catch(() =>
          invoke<AppState>("get_app_state")
        )
      : invoke<AppState>("get_app_state");

    void stateRequest.then((state) => {
      setAppState(state);
      setTargetDirectory(state.customSongsPath);
      localStorage.setItem("unchartable:custom-songs", state.customSongsPath);
    });
  }, []);

  const loadCharts = useCallback(async () => {
    setLoading(true);
    setCatalogError("");
    try {
      setCatalog(await fetchCatalog(query, page, difficulty, rankedOnly));
    } catch (error) {
      setCatalogError(error instanceof Error ? error.message : "Could not load charts.");
    } finally {
      setLoading(false);
    }
  }, [difficulty, page, query, rankedOnly]);

  useEffect(() => {
    void loadCharts();
  }, [loadCharts]);

  const refreshLibrary = useCallback(async () => {
    if (!isTauri() || !targetDirectory) return;
    setLibraryLoading(true);
    try {
      const [installed, trash] = await Promise.all([
        invoke<InstalledChart[]>("list_installed_charts", { path: targetDirectory }),
        invoke<TrashItem[]>("list_trashed_charts", { path: targetDirectory })
      ]);
      setInstalledCharts(installed);
      setTrashItems(trash);
      setLibraryLoading(false);
      void invoke<ManualChartMatch[]>("find_manual_chart_matches", { path: targetDirectory })
        .then(setManualMatches)
        .catch(() => setManualMatches([]));
    } catch (error) {
      setUpdateMessage(`Could not scan CustomSongs: ${String(error)}`);
      setLibraryLoading(false);
    }
  }, [targetDirectory]);

  useEffect(() => {
    void refreshLibrary();
  }, [refreshLibrary]);

  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    void listen<InstallProgress>("install-progress", ({ payload }) => {
      setInstalling((current) => {
        const existing = current[payload.chartId];
        return existing ? { ...current, [payload.chartId]: { ...existing, progress: payload } } : current;
      });
    }).then((cleanup) => {
      unlisten = cleanup;
    });
    return () => unlisten?.();
  }, []);

  const activeInstalls = useMemo(
    () => Object.values(installing).filter((entry) => entry.progress.stage !== "complete" && !entry.error).length,
    [installing]
  );
  const installedIds = useMemo(
    () => new Set(installedCharts.flatMap((chart) => chart.chartId ? [chart.chartId] : [])),
    [installedCharts]
  );
  const manualMatchesByPath = useMemo(
    () => new Map(manualMatches.map((match) => [match.installedPath, match])),
    [manualMatches]
  );
  const visibleInstalledCharts = useMemo(() => {
    const needle = installedQuery.trim().toLocaleLowerCase();
    if (!needle) return installedCharts;
    return installedCharts.filter((chart) =>
      [chart.title, chart.artist, chart.charter, chart.folderName]
        .filter(Boolean)
        .some((value) => value!.toLocaleLowerCase().includes(needle))
    );
  }, [installedCharts, installedQuery]);

  function togglePreview(chart: Chart) {
    const current = previewAudioRef.current;
    if (previewingId === chart.id && current) {
      current.pause();
      previewAudioRef.current = null;
      setPreviewingId(null);
      return;
    }

    current?.pause();
    const audio = new Audio(buildChartPreviewUrl(chart));
    audio.preload = "none";
    audio.volume = 0.7;
    audio.onended = () => {
      if (previewAudioRef.current !== audio) return;
      previewAudioRef.current = null;
      setPreviewingId(null);
    };
    audio.onerror = () => {
      if (previewAudioRef.current !== audio) return;
      previewAudioRef.current = null;
      setPreviewingId(null);
      setPreviewErrorId(chart.id);
    };
    previewAudioRef.current = audio;
    setPreviewErrorId(null);
    void audio.play().then(() => setPreviewingId(chart.id)).catch(() => {
      if (previewAudioRef.current === audio) previewAudioRef.current = null;
      setPreviewingId(null);
      setPreviewErrorId(chart.id);
    });
  }

  async function chooseDirectory() {
    if (!isTauri()) return;
    const selected = await open({ directory: true, multiple: false, title: "Choose UNBEATABLE CustomSongs folder" });
    if (!selected || Array.isArray(selected)) return;
    try {
      const state = await invoke<AppState>("validate_custom_songs_path", { path: selected });
      setTargetDirectory(state.customSongsPath);
      setAppState(state);
      localStorage.setItem("unchartable:custom-songs", state.customSongsPath);
    } catch (error) {
      setAppState((current) => current ? { ...current, directoryExists: false } : current);
      window.alert(String(error));
    }
  }

  async function openTargetDirectory() {
    if (!isTauri() || !targetDirectory) return;
    try {
      await invoke("open_custom_songs_folder", { path: targetDirectory });
    } catch (error) {
      window.alert(`Could not open the CustomSongs folder.\n\n${String(error)}`);
    }
  }

  async function install(chart: Chart) {
    if (!isTauri()) return false;
    if (!targetDirectory) {
      setView("settings");
      return false;
    }
    const initialProgress: InstallProgress = {
      chartId: chart.id,
      downloadedBytes: 0,
      stage: "requesting",
      totalBytes: null
    };
    setInstalling((current) => ({ ...current, [chart.id]: { chart, progress: initialProgress } }));
    try {
      const result = await invoke<InstallResult>("install_chart", {
        artist: chart.artist,
        chartId: chart.id,
        charter: chart.charterName,
        targetDirectory,
        title: chart.title,
        updatedAt: chartContentVersion(chart)
      });
      setInstalling((current) => ({
        ...current,
        [chart.id]: {
          chart,
          installPath: result.installPath,
          progress: { ...initialProgress, stage: "complete" }
        }
      }));
      await refreshLibrary();
      setInstalling((current) => {
        const next = { ...current };
        delete next[chart.id];
        return next;
      });
      return true;
    } catch (error) {
      setInstalling((current) => ({
        ...current,
        [chart.id]: {
          chart,
          error: String(error),
          progress: { ...initialProgress, stage: "failed" }
        }
      }));
      return false;
    }
  }

  useEffect(() => {
    if (!pendingInstallId || !appState) return;
    const chartId = pendingInstallId;
    setPendingInstallId(null);

    if (!targetDirectory) {
      setView("settings");
      setUpdateMessage("Choose your UNBEATABLE CustomSongs folder to install this chart.");
      return;
    }
    if (installedIds.has(chartId)) {
      setView("downloads");
      setUpdateMessage("This chart is already installed.");
      return;
    }
    if (installing[chartId]) {
      setView("downloads");
      return;
    }

    setView("downloads");
    void fetchChart(chartId)
      .then((chart) => install(chart))
      .catch((error) => {
        setView("charts");
        setUpdateMessage(error instanceof Error ? error.message : "Could not open this chart.");
      });
  }, [appState, installedIds, installing, pendingInstallId, targetDirectory]);

  useEffect(() => {
    if (!isTauri() || !automaticUpdates || !appState?.directoryExists || !targetDirectory) return;
    let cancelled = false;
    const checkForUpdates = async () => {
      try {
        const updates = await invoke<UpdateCandidate[]>("check_installed_updates", { path: targetDirectory });
        if (cancelled) return;
        if (updates.length === 0) return;
        let completed = 0;
        for (const update of updates) {
          if (cancelled) return;
          if (await install(update.chart)) completed += 1;
        }
        if (!cancelled && completed === updates.length) setUpdateMessage("");
      } catch (error) {
        if (!cancelled) setUpdateMessage(`Automatic update check failed: ${String(error)}`);
      }
    };
    void checkForUpdates();
    const interval = window.setInterval(() => void checkForUpdates(), 30 * 60 * 1000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, [appState?.directoryExists, automaticUpdates, targetDirectory]);

  async function removeInstalled(chart: InstalledChart) {
    if (!chart.chartId || !chart.managed || !targetDirectory) return;
    if (!window.confirm(`Move "${chart.title}" to UNCHARTABLE trash?`)) return;
    try {
      await invoke("trash_installed_chart", { path: targetDirectory, chartId: chart.chartId });
      setUpdateMessage(`${chart.title} moved to trash.`);
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not remove ${chart.title}: ${String(error)}`);
    }
  }

  async function restoreTrash(item: TrashItem) {
    if (!targetDirectory) return;
    try {
      await invoke("restore_trashed_chart", { path: targetDirectory, trashId: item.trashId });
      setUpdateMessage(`${item.title} restored.`);
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not restore ${item.title}: ${String(error)}`);
    }
  }

  async function emptyTrash() {
    if (!targetDirectory || trashItems.length === 0) return;
    if (!window.confirm(`Permanently delete ${trashItems.length} chart${trashItems.length === 1 ? "" : "s"} from trash?`)) return;
    try {
      const removed = await invoke<number>("empty_chart_trash", { path: targetDirectory });
      setUpdateMessage(`${removed} trashed chart${removed === 1 ? "" : "s"} permanently deleted.`);
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not empty trash: ${String(error)}`);
    }
  }

  async function adoptManualChart(chart: InstalledChart, match: ManualChartMatch) {
    if (!targetDirectory) return;
    try {
      await invoke("adopt_manual_chart", {
        chartId: match.chart.id,
        installedPath: chart.path,
        path: targetDirectory
      });
      setUpdateMessage("");
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not enable updates for ${chart.title}: ${String(error)}`);
    }
  }

  async function setChartUpdates(chart: InstalledChart, enabled: boolean) {
    if (!targetDirectory || !chart.chartId) return;
    setInstalledCharts((current) =>
      current.map((item) => item.path === chart.path ? { ...item, updatesEnabled: enabled } : item)
    );
    try {
      await invoke("set_chart_updates", {
        chartId: chart.chartId,
        enabled,
        installedPath: chart.path,
        path: targetDirectory
      });
      setUpdateMessage("");
    } catch (error) {
      setInstalledCharts((current) =>
        current.map((item) => item.path === chart.path ? { ...item, updatesEnabled: !enabled } : item)
      );
      setUpdateMessage(`Could not change updates for ${chart.title}: ${String(error)}`);
    }
  }

  async function launchGame() {
    try {
      await invoke("launch_unbeatable");
    } catch (error) {
      setUpdateMessage(`Could not launch UNBEATABLE: ${String(error)}`);
    }
  }

  async function cancelInstall(chartId: string) {
    try {
      await invoke("cancel_install", { chartId });
      setUpdateMessage("Cancelling the installation...");
    } catch (error) {
      setUpdateMessage(`Could not cancel the installation: ${String(error)}`);
    }
  }

  function clearFinishedDownloads() {
    setInstalling((current) => Object.fromEntries(
      Object.entries(current).filter(([, entry]) => entry.progress.stage !== "complete" && !entry.error)
    ));
  }

  return (
    <div className="app-frame">
      <aside className="sidebar">
        <button className="brand" onClick={() => setView("charts")} type="button">
          <img alt="" src="/unchartable.png" />
          <span>UNCHARTABLE</span>
        </button>
        <nav aria-label="Application">
          <NavButton active={view === "charts"} icon={<Library />} label="charts" onClick={() => setView("charts")} />
          <NavButton
            active={view === "downloads"}
            badge={activeInstalls || undefined}
            icon={<Download />}
            label="downloads"
            onClick={() => setView("downloads")}
          />
          <NavButton active={view === "settings"} icon={<Settings />} label="settings" onClick={() => setView("settings")} />
        </nav>
        <div className="sidebar-foot">
          <div className={appState?.directoryExists ? "path-state path-state-ready" : "path-state"}>
            {appState?.directoryExists ? <Check /> : <Folder />}
            <div>
              <strong>{appState?.directoryExists ? "game folder ready" : "folder not confirmed"}</strong>
              <span>{targetDirectory || "choose CustomSongs"}</span>
            </div>
          </div>
          <button className="text-button" onClick={() => void openExternalUrl(API_ORIGIN)} type="button">
            unchartable.site <ExternalLink />
          </button>
          <button className="launch-button" onClick={() => void launchGame()} type="button">
            <Gamepad2 /> play UNBEATABLE
          </button>
        </div>
      </aside>

      <main>
        {view === "charts" ? (
          <>
            <header className="page-header">
              <div>
                <p className="kicker">community charts</p>
                <h1>browse and install</h1>
              </div>
              <div className="catalog-total">{catalog.count} charts</div>
            </header>

            <section className="filter-bar">
              <label className="search-field">
                <Search />
                <input
                  aria-label="Search charts"
                  onChange={(event) => setQueryInput(event.target.value)}
                  placeholder="search title, artist, or charter"
                  value={queryInput}
                />
                {queryInput ? (
                  <button aria-label="Clear search" onClick={() => setQueryInput("")} type="button"><X /></button>
                ) : null}
              </label>
              <label className="select-field">
                <SlidersHorizontal />
                <select
                  aria-label="Difficulty"
                  onChange={(event) => {
                    setDifficulty(event.target.value);
                    setPage(0);
                  }}
                  value={difficulty}
                >
                  {difficulties.map((value) => <option key={value || "all"} value={value}>{value || "all difficulties"}</option>)}
                </select>
              </label>
              <button
                aria-pressed={rankedOnly}
                className={rankedOnly ? "toggle-button toggle-button-active" : "toggle-button"}
                onClick={() => {
                  setRankedOnly((current) => !current);
                  setPage(0);
                }}
                type="button"
              >
                <Gauge /> ranked
              </button>
            </section>

            {!appState?.directoryExists ? (
              <button className="setup-banner" onClick={() => setView("settings")} type="button">
                <FolderOpen />
                <span><strong>Confirm your CustomSongs folder</strong> before installing your first chart.</span>
                <ChevronRight />
              </button>
            ) : null}

            {catalogError ? (
              <section className="empty-state">
                <ShieldCheck />
                <h2>could not reach the chart catalog</h2>
                <p>{catalogError}</p>
                <button onClick={() => void loadCharts()} type="button">try again</button>
              </section>
            ) : loading ? (
              <section className="loading-state"><LoaderCircle /><span>loading charts</span></section>
            ) : catalog.charts.length === 0 ? (
              <section className="empty-state"><Search /><h2>no charts found</h2><p>Try a different title, artist, or difficulty.</p></section>
            ) : (
              <section className="chart-grid">
                {catalog.charts.map((chart) => (
                  <ChartCard
                    chart={chart}
                    entry={installing[chart.id]}
                    installed={installedIds.has(chart.id)}
                    key={chart.id}
                    onInstall={() => void install(chart)}
                    onPreview={() => togglePreview(chart)}
                    previewError={previewErrorId === chart.id}
                    previewing={previewingId === chart.id}
                  />
                ))}
              </section>
            )}

            <footer className="pagination">
              <button disabled={page === 0 || loading} onClick={() => setPage((current) => Math.max(0, current - 1))} type="button">
                <ChevronLeft /> previous
              </button>
              <span>page {page + 1}</span>
              <button disabled={catalog.nextPage === null || loading} onClick={() => setPage(catalog.nextPage ?? page)} type="button">
                next <ChevronRight />
              </button>
            </footer>
          </>
        ) : null}

        {view === "downloads" ? (
          <>
            <header className="page-header">
              <div><p className="kicker">downloads and CustomSongs</p><h1>your charts</h1></div>
              <div className="download-header-actions">
                <span className="catalog-total">{activeInstalls} active</span>
                <button disabled={libraryLoading} onClick={() => void refreshLibrary()} type="button">
                  <RefreshCw className={libraryLoading ? "spin" : ""} /> refresh
                </button>
                {Object.values(installing).some((entry) => entry.progress.stage === "complete" || entry.error) ? (
                  <button onClick={clearFinishedDownloads} type="button"><Trash2 /> clear finished</button>
                ) : null}
              </div>
            </header>
            {updateMessage ? <div className="status-banner"><PackageCheck /> {updateMessage}</div> : null}
            {Object.values(installing).length > 0 ? (
              <section className="library-section">
                <div className="section-heading">
                  <div><h2>download queue</h2></div>
                  <span>{Object.values(installing).length}</span>
                </div>
                <div className="download-list">
                  {Object.values(installing).reverse().map((entry) => (
                    <DownloadRow
                      entry={entry}
                      key={entry.chart.id}
                      onCancel={() => void cancelInstall(entry.chart.id)}
                      onRetry={() => void install(entry.chart)}
                    />
                  ))}
                </div>
              </section>
            ) : null}
            <section className="library-section">
              <div className="section-heading">
                <div><h2>installed</h2></div>
                <div className="installed-heading-tools">
                  <label className="installed-search">
                    <Search />
                    <input
                      aria-label="Search installed charts"
                      onChange={(event) => setInstalledQuery(event.target.value)}
                      placeholder="search installed charts"
                      type="search"
                      value={installedQuery}
                    />
                  </label>
                  <span>{visibleInstalledCharts.length}</span>
                </div>
              </div>
            {libraryLoading && installedCharts.length === 0 ? (
              <section className="loading-state"><LoaderCircle /><span>scanning CustomSongs</span></section>
            ) : installedCharts.length === 0 ? (
              <section className="empty-state">
                <HardDrive /><h2>no charts found</h2><p>Install a chart or refresh after adding one manually.</p>
              </section>
            ) : visibleInstalledCharts.length === 0 ? (
              <section className="empty-state compact-empty">
                <Search /><h2>no matching charts</h2><p>Try another title, artist, charter, or folder name.</p>
              </section>
            ) : (
              <section className="installed-list">
                {visibleInstalledCharts.map((chart) => {
                  const manualMatch = manualMatchesByPath.get(chart.path);
                  return (
                  <article className="installed-row" key={chart.path}>
                    <InstalledCover chart={chart} match={manualMatch} />
                    <div className="installed-copy">
                      <div className="installed-title">
                        <h2>{chart.title}</h2>
                      </div>
                      <p>
                        {[chart.artist, chart.charter ? `charted by ${chart.charter}` : null].filter(Boolean).join(" · ") || chart.folderName}
                      </p>
                      <small>
                        {formatBytes(chart.sizeBytes)} · {chart.playable ? "chart files found" : "missing chart text or audio"}
                        {chart.installedAt ? ` · installed ${new Date(chart.installedAt).toLocaleDateString()}` : ""}
                      </small>
                      {manualMatch ? (
                        <small className="manual-match">
                          <Link2 />
                          found on UNCHARTABLE as {manualMatch.chart.title} by {manualMatch.chart.charterName}
                        </small>
                      ) : null}
                    </div>
                    <div className="installed-actions">
                      <button aria-label="Open CustomSongs folder" onClick={() => void openTargetDirectory()} title="open folder" type="button">
                        <FolderOpen />
                      </button>
                      {chart.managed ? (
                        <button
                          aria-label={`${chart.updatesEnabled ? "Disable" : "Enable"} updates for ${chart.title}`}
                          className={chart.updatesEnabled ? "updates-icon updates-icon-on" : "updates-icon"}
                          onClick={() => void setChartUpdates(chart, !chart.updatesEnabled)}
                          title={chart.updatesEnabled ? "automatic updates on" : "automatic updates off"}
                          type="button"
                        >
                          <RefreshCw />
                        </button>
                      ) : manualMatch ? (
                        <button
                          aria-label={`Enable automatic updates for ${chart.title}`}
                          className="adopt-sync-icon"
                          onClick={() => void adoptManualChart(chart, manualMatch)}
                          title="enable automatic updates"
                          type="button"
                        >
                          <RefreshCw />
                        </button>
                      ) : null}
                      {chart.managed ? (
                        <button
                          aria-label={`Move ${chart.title} to trash`}
                          className="danger-icon"
                          onClick={() => void removeInstalled(chart)}
                          title="move to trash"
                          type="button"
                        >
                          <Trash2 />
                        </button>
                      ) : null}
                    </div>
                  </article>
                  );
                })}
              </section>
            )}
            </section>
            {trashItems.length > 0 ? (
              <section className="trash-section">
                <div className="section-heading">
                  <div><p className="kicker">recoverable</p><h2>trash</h2></div>
                  <div className="section-heading-actions">
                    <span>{trashItems.length}</span>
                    <button onClick={() => void emptyTrash()} type="button"><Trash2 /> empty trash</button>
                  </div>
                </div>
                <div className="trash-list">
                  {trashItems.map((item) => (
                    <article className="trash-row" key={item.trashId}>
                      <Trash2 />
                      <div><strong>{item.title}</strong><small>{formatBytes(item.sizeBytes)} · deleted {new Date(item.deletedAt).toLocaleString()}</small></div>
                      <button onClick={() => void restoreTrash(item)} type="button"><RotateCcw /> restore</button>
                    </article>
                  ))}
                </div>
              </section>
            ) : null}
          </>
        ) : null}

        {view === "settings" ? (
          <>
            <header className="page-header">
              <div><p className="kicker">application setup</p><h1>settings</h1></div>
            </header>
            <section className="settings-panel">
              <div className="setting-row">
                <div className="setting-copy">
                  {theme === "dark" ? <Moon /> : <Sun />}
                  <div>
                    <h2>appearance</h2>
                    <p>Choose one visual mode. Your preference stays on this device.</p>
                  </div>
                </div>
                <div className="theme-selector" aria-label="Appearance">
                  <button
                    aria-pressed={theme === "light"}
                    className={theme === "light" ? "theme-option theme-option-active" : "theme-option"}
                    onClick={() => setTheme("light")}
                    type="button"
                  >
                    <Sun /> light
                  </button>
                  <button
                    aria-pressed={theme === "dark"}
                    className={theme === "dark" ? "theme-option theme-option-active" : "theme-option"}
                    onClick={() => setTheme("dark")}
                    type="button"
                  >
                    <Moon /> dark
                  </button>
                </div>
              </div>
              <div className="settings-divider" />
              <div className="setting-row">
                <div className="setting-copy">
                  <RefreshCw />
                  <div>
                    <h2>automatic chart updates</h2>
                    <p>Replace managed charts when UNCHARTABLE publishes a newer content version. Previous files are kept until the update succeeds.</p>
                  </div>
                </div>
                <button
                  aria-pressed={automaticUpdates}
                  className={automaticUpdates ? "switch-control switch-control-on" : "switch-control"}
                  onClick={() => {
                    const next = !automaticUpdates;
                    setAutomaticUpdates(next);
                    localStorage.setItem("unchartable:auto-updates", next ? "on" : "off");
                  }}
                  type="button"
                >
                  <span /> {automaticUpdates ? "on" : "off"}
                </button>
              </div>
              {updateMessage ? <div className="inline-status"><PackageCheck /> {updateMessage}</div> : null}
              <div className="settings-divider" />
              <div className="setting-copy">
                <FolderOpen />
                <div>
                  <h2>UNBEATABLE CustomSongs</h2>
                  <p>UNCHARTABLE extracts verified chart archives directly into this folder.</p>
                </div>
              </div>
              <div className="folder-control">
                <code>{targetDirectory || "No folder selected"}</code>
                <button onClick={() => void chooseDirectory()} type="button"><Folder /> choose folder</button>
                {targetDirectory && isTauri() ? (
                  <button className="secondary-button" onClick={() => void openTargetDirectory()} type="button">
                    <ExternalLink /> open folder
                  </button>
                ) : null}
              </div>
              <div className="security-note">
                <ShieldCheck />
                <p>Chart ZIPs are downloaded securely, checked for unsafe files, and installed only after verification.</p>
              </div>
            </section>
          </>
        ) : null}
      </main>
    </div>
  );
}

function NavButton({ active, badge, icon, label, onClick }: { active: boolean; badge?: number; icon: React.ReactNode; label: string; onClick: () => void }) {
  return (
    <button aria-current={active ? "page" : undefined} className={active ? "nav-button nav-button-active" : "nav-button"} onClick={onClick} type="button">
      {icon}<span>{label}</span>{badge ? <small>{badge}</small> : null}
    </button>
  );
}

function InstalledCover({ chart, match }: { chart: InstalledChart; match?: ManualChartMatch }) {
  const [imageFailed, setImageFailed] = useState(false);
  const coverChart = match?.chart ?? (
    chart.chartId
      ? { id: chart.chartId, updatedAt: chart.updatedAt || chart.chartId }
      : null
  );
  const coverUrl = coverChart ? buildChartCoverUrl(coverChart) : null;

  useEffect(() => setImageFailed(false), [coverUrl]);

  return (
    <div className={chart.playable ? "installed-cover installed-cover-ready" : "installed-cover installed-cover-warning"}>
      {coverUrl && !imageFailed ? (
        <img
          alt=""
          loading="lazy"
          onError={() => setImageFailed(true)}
          src={coverUrl}
        />
      ) : chart.playable ? <PackageCheck /> : <ShieldCheck />}
    </div>
  );
}

function installProgressPercent(progress?: InstallProgress) {
  if (!progress) return 0;
  if (progress.stage === "requesting") return 6;
  if (progress.stage === "installing") return 94;
  if (progress.stage === "complete") return 100;
  return progress.totalBytes
    ? Math.min(92, Math.round((progress.downloadedBytes / progress.totalBytes) * 92))
    : 12;
}

function DownloadProgressRing({ percent }: { percent: number }) {
  return (
    <span
      aria-hidden="true"
      className="download-progress-ring"
      style={{ "--download-progress": `${Math.max(0, Math.min(100, percent))}%` } as CSSProperties}
    >
      <Download />
    </span>
  );
}

function ChartCard({
  chart,
  entry,
  installed,
  onInstall,
  onPreview,
  previewError,
  previewing
}: {
  chart: Chart;
  entry?: InstallEntry;
  installed: boolean;
  onInstall: () => void;
  onPreview: () => void;
  previewError: boolean;
  previewing: boolean;
}) {
  const progress = entry?.progress;
  const progressPercent = installProgressPercent(progress);
  const complete = progress?.stage === "complete" || installed;
  const pending = Boolean(progress && !complete && progress.stage !== "failed");
  return (
    <article className="chart-card">
      <div className="cover-wrap">
        <img alt="" loading="lazy" src={buildChartCoverUrl(chart)} />
        {chart.rankedStatus === "ranked" ? <span className="ranked-tag">ranked</span> : null}
      </div>
      <div className="chart-copy">
        <div>
          <div className="chart-title-row">
            <h2>{chart.title}</h2>
            <span className={difficultyClass(chart.difficulty)}>{chart.difficultyLevel} {chart.difficulty}</span>
          </div>
          <p className="artist">{chart.artist}</p>
          <p className="charter">charted by <strong>{chart.submitter?.displayName || chart.charterName}</strong></p>
        </div>
        <div className="difficulty-row">
          {chart.difficultyLevels.map((level, index) => (
            <span className={difficultyClass(level.difficulty)} key={`${level.difficulty}-${level.level}-${index}`}>
              {level.level} {level.difficulty}
            </span>
          ))}
        </div>
        <div className="chart-meta">
          <span>{formatDuration(chart.audioDurationSeconds)}</span>
          <span>{chart.downloadCount} downloads</span>
        </div>
        <div className="card-status-slot">
          {entry?.error ? <p className="card-error">{entry.error}</p> : null}
        </div>
        <div className="card-actions">
          <button className={complete ? "install-button install-button-complete" : "install-button"} disabled={pending || complete || !chart.hasDirectDownload} onClick={onInstall} type="button">
            {pending ? <DownloadProgressRing percent={progressPercent} /> : complete ? <Check /> : <Download />}
            {complete ? "installed" : pending ? `${progressPercent}%` : chart.hasDirectDownload ? "install" : "unavailable"}
          </button>
          <button
            aria-label={`${previewing ? "Pause" : "Play"} preview for ${chart.title}`}
            className={`icon-button preview-button${previewing ? " preview-button-active" : ""}${previewError ? " preview-button-error" : ""}`}
            onClick={onPreview}
            title={previewError ? "preview unavailable - click to retry" : previewing ? "pause preview" : "play preview"}
            type="button"
          >
            {previewing ? <Pause /> : <Play />}
          </button>
          <button aria-label={`Open ${chart.title} on unchartable.site`} className="icon-button" onClick={() => void openExternalUrl(buildChartPublicUrl(chart))} title="open on unchartable.site" type="button">
            <ExternalLink />
          </button>
        </div>
      </div>
    </article>
  );
}

function DownloadRow({ entry, onCancel, onRetry }: { entry: InstallEntry; onCancel: () => void; onRetry: () => void }) {
  const { progress } = entry;
  const percent = installProgressPercent(progress);
  return (
    <article className="download-row">
      <img alt="" src={buildChartCoverUrl(entry.chart)} />
      <div>
        <h2>{entry.chart.title}</h2>
        <p>{entry.chart.artist}</p>
        <small>
          {entry.error
            ? entry.error
            : progress.stage === "complete"
              ? `installed in ${entry.installPath}`
              : `${progress.stage} · ${formatBytes(progress.downloadedBytes)}${progress.totalBytes ? ` / ${formatBytes(progress.totalBytes)}` : ""}`}
        </small>
      </div>
      <div className="download-row-actions">
        {entry.error ? (
          <button aria-label={`Retry ${entry.chart.title}`} onClick={onRetry} title="retry" type="button"><RefreshCw /></button>
        ) : progress.stage === "complete" ? (
          <Check className="complete-icon" />
        ) : (
          <>
            <DownloadProgressRing percent={percent} />
            <button aria-label={`Cancel ${entry.chart.title}`} onClick={onCancel} title="cancel" type="button"><X /></button>
          </>
        )}
      </div>
    </article>
  );
}

export default App;
