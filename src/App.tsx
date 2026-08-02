import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Archive,
  Bug,
  Check,
  CheckSquare,
  ChevronLeft,
  ChevronRight,
  CircleAlert,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  Gauge,
  Gamepad2,
  HardDrive,
  Hammer,
  History,
  Library,
  Link2,
  LoaderCircle,
  Moon,
  Package,
  PackageCheck,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Search,
  Settings,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  Sun,
  Trash2,
  Undo2,
  Upload,
  Wrench,
  X
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import "./App.css";
import appPackage from "../package.json";
import {
  API_ORIGIN,
  buildBugReportUrl,
  buildChartCoverUrl,
  buildChartPreviewUrl,
  buildChartPublicUrl,
  chartMatchesInstalledChart,
  chartContentVersion,
  difficultyClass,
  formatBytes,
  formatDuration,
  isArchiveDrop,
  parseInstallDeepLink,
  type AppState,
  type BackupItem,
  type Chart,
  type ChartCatalog,
  type ChartPack,
  type DiagnosticReport,
  type ImportArchiveInspection,
  type InstallProgress,
  type InstallResult,
  type InstalledChart,
  type LocalPack,
  type ManualChartMatch,
  type PackCatalog,
  type RepairReport,
  type TrashItem,
  type UpdateCandidate
} from "./lib";

type View = "charts" | "packs" | "downloads" | "updates" | "settings";
type Theme = "light" | "dark";
type InstallEntry = {
  chart: Chart;
  error?: string;
  installPath?: string;
  progress: InstallProgress;
};

const difficulties = ["", "beginner", "normal", "hard", "expert", "UNBEATABLE", "STAR"];
const isTauri = () => "__TAURI_INTERNALS__" in window;
const matchAdoptionBatchSize = 4;
const installQueueStorageKey = "unchartable:install-queue";
const localPacksStorageKey = "unchartable:local-packs";
const apiCachePrefix = "unchartable:api-cache:";
const apiCacheLifetime = 5 * 60 * 1000;

function readApiCache<T>(key: string): T | null {
  try {
    const cached = JSON.parse(localStorage.getItem(`${apiCachePrefix}${key}`) || "null") as {
      expiresAt: number;
      value: T;
    } | null;
    return cached && cached.expiresAt > Date.now() ? cached.value : null;
  } catch {
    return null;
  }
}

function writeApiCache<T>(key: string, value: T) {
  try {
    localStorage.setItem(`${apiCachePrefix}${key}`, JSON.stringify({
      expiresAt: Date.now() + apiCacheLifetime,
      value
    }));
  } catch {
    // Cache failure should never block the app.
  }
}

function readLocalPacks(): LocalPack[] {
  try {
    const value = JSON.parse(localStorage.getItem(localPacksStorageKey) || "[]");
    return Array.isArray(value) ? value : [];
  } catch {
    return [];
  }
}

function readPersistedQueue(): Record<string, InstallEntry> {
  try {
    const value = JSON.parse(localStorage.getItem(installQueueStorageKey) || "{}") as Record<string, InstallEntry>;
    return Object.fromEntries(Object.entries(value).map(([id, entry]) => [
      id,
      {
        ...entry,
        error: entry.progress.stage === "complete" ? undefined : "download paused when the app closed",
        progress: entry.progress.stage === "complete"
          ? entry.progress
          : { ...entry.progress, stage: "failed" as const }
      }
    ]));
  } catch {
    return {};
  }
}

async function openExternalUrl(url: string) {
  if (isTauri()) {
    await invoke("open_external_url", { url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

async function fetchCatalog(query: string, page: number, difficulty: string, rankedOnly: boolean) {
  const cacheKey = `charts:${query}:${page}:${difficulty}:${rankedOnly}`;
  const cached = readApiCache<ChartCatalog>(cacheKey);
  if (cached) return cached;
  if (isTauri()) {
    const result = await invoke<ChartCatalog>("fetch_charts", { difficulty, page, query, rankedOnly });
    writeApiCache(cacheKey, result);
    return result;
  }
  const params = new URLSearchParams({ page: String(page), pageSize: "24", sort: "newest" });
  if (query) params.set("q", query);
  if (difficulty) params.set("difficulty", difficulty);
  if (rankedOnly) params.set("ranked", "1");
  const response = await fetch(`${API_ORIGIN}/api/charts?${params}`);
  if (!response.ok) throw new Error("Could not load charts.");
  const result = await response.json() as ChartCatalog;
  writeApiCache(cacheKey, result);
  return result;
}

async function fetchChart(chartId: string) {
  if (isTauri()) return invoke<Chart>("fetch_chart", { chartId });
  const response = await fetch(`${API_ORIGIN}/api/charts/${encodeURIComponent(chartId)}`);
  const payload = await response.json() as { chart?: Chart };
  if (!response.ok || !payload.chart) throw new Error("Could not load this chart.");
  return payload.chart;
}

async function fetchPacks(query: string, page: number) {
  const cacheKey = `packs:full:${query}:${page}`;
  const cached = readApiCache<PackCatalog>(cacheKey);
  if (cached) return cached;
  if (isTauri()) {
    const result = await invoke<PackCatalog>("fetch_packs", { page, query });
    writeApiCache(cacheKey, result);
    return result;
  }
  const params = new URLSearchParams({ page: String(page), pageSize: "12" });
  params.set("charts", "all");
  if (query) params.set("q", query);
  const response = await fetch(`${API_ORIGIN}/api/packs?${params}`);
  if (!response.ok) throw new Error("Could not load packs.");
  const result = await response.json() as PackCatalog;
  writeApiCache(cacheKey, result);
  return result;
}

async function fetchPack(packId: string) {
  if (isTauri()) return invoke<ChartPack>("fetch_pack", { packId });
  const response = await fetch(`${API_ORIGIN}/api/packs/${encodeURIComponent(packId)}`);
  const payload = await response.json() as { pack?: ChartPack };
  if (!response.ok || !payload.pack) throw new Error("Could not load this pack.");
  return payload.pack;
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
  const [installing, setInstalling] = useState<Record<string, InstallEntry>>(readPersistedQueue);
  const [pendingInstallId, setPendingInstallId] = useState<string | null>(null);
  const [installedCharts, setInstalledCharts] = useState<InstalledChart[]>([]);
  const [installedQuery, setInstalledQuery] = useState("");
  const [manualMatches, setManualMatches] = useState<ManualChartMatch[]>([]);
  const [trashItems, setTrashItems] = useState<TrashItem[]>([]);
  const [libraryLoading, setLibraryLoading] = useState(false);
  const [bulkUpdatesLoading, setBulkUpdatesLoading] = useState(false);
  const [updateCandidates, setUpdateCandidates] = useState<UpdateCandidate[]>([]);
  const [updatesLoading, setUpdatesLoading] = useState(false);
  const [backups, setBackups] = useState<BackupItem[]>([]);
  const [diagnostic, setDiagnostic] = useState<DiagnosticReport | null>(null);
  const [diagnosticLoading, setDiagnosticLoading] = useState(false);
  const [repairReport, setRepairReport] = useState<RepairReport | null>(null);
  const [repairLoading, setRepairLoading] = useState(false);
  const [importInspections, setImportInspections] = useState<ImportArchiveInspection[]>([]);
  const [importLoading, setImportLoading] = useState(false);
  const [dropState, setDropState] = useState<"valid" | "invalid" | null>(null);
  const [localPacks, setLocalPacks] = useState<LocalPack[]>(readLocalPacks);
  const [appVisible, setAppVisible] = useState(() => !document.hidden);
  const [installedVisibleLimit, setInstalledVisibleLimit] = useState(40);
  const [selectedInstalled, setSelectedInstalled] = useState<Set<string>>(new Set());
  const [packCatalog, setPackCatalog] = useState<PackCatalog>({ packs: [], count: 0, nextPage: null });
  const [packQueryInput, setPackQueryInput] = useState("");
  const [packQuery, setPackQuery] = useState("");
  const [packPage, setPackPage] = useState(0);
  const [packsLoading, setPacksLoading] = useState(false);
  const [packError, setPackError] = useState("");
  const [packSelections, setPackSelections] = useState<Record<string, Set<string>>>({});
  const [updateMessage, setUpdateMessage] = useState("");
  const [previewingId, setPreviewingId] = useState<string | null>(null);
  const [previewErrorId, setPreviewErrorId] = useState<string | null>(null);
  const previewAudioRef = useRef<HTMLAudioElement | null>(null);
  const [automaticUpdates, setAutomaticUpdates] = useState(
    () => localStorage.getItem("unchartable:auto-updates") !== "off"
  );

  const inspectArchives = useCallback(async (archivePaths: string[]) => {
    if (!isTauri() || !targetDirectory || importLoading) return;
    setImportLoading(true);
    try {
      const inspections: ImportArchiveInspection[] = [];
      const failures: string[] = [];
      for (const archivePath of [...new Set(archivePaths)]) {
        try {
          inspections.push(await invoke<ImportArchiveInspection>("inspect_chart_archive", {
            archivePath,
            targetDirectory
          }));
        } catch (error) {
          failures.push(`${archivePath.split(/[\\/]/).pop() || "archive"}: ${String(error)}`);
        }
      }
      setImportInspections(inspections);
      setUpdateMessage(
        failures.length
          ? `${failures.length} archive${failures.length === 1 ? "" : "s"} could not be validated. ${failures[0]}`
          : ""
      );
    } catch (error) {
      setImportInspections([]);
      setUpdateMessage(`Import validation failed: ${String(error)}`);
    } finally {
      setImportLoading(false);
    }
  }, [importLoading, targetDirectory]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem("unchartable:theme", theme);
  }, [theme]);

  useEffect(() => {
    localStorage.setItem(installQueueStorageKey, JSON.stringify(installing));
  }, [installing]);

  useEffect(() => {
    localStorage.setItem(localPacksStorageKey, JSON.stringify(localPacks));
  }, [localPacks]);

  useEffect(() => {
    const handleVisibility = () => {
      const visible = !document.hidden;
      setAppVisible(visible);
      if (!visible) {
        previewAudioRef.current?.pause();
        previewAudioRef.current = null;
        setPreviewingId(null);
      }
    };
    document.addEventListener("visibilitychange", handleVisibility);
    return () => document.removeEventListener("visibilitychange", handleVisibility);
  }, []);

  useEffect(() => () => {
    previewAudioRef.current?.pause();
    previewAudioRef.current = null;
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent((event) => {
      if (disposed) return;
      if (event.payload.type === "enter") {
        setDropState(isArchiveDrop(event.payload.paths) ? "valid" : "invalid");
        return;
      }
      if (event.payload.type === "over") return;
      if (event.payload.type === "leave") {
        setDropState(null);
        return;
      }
      setDropState(null);
      setView("downloads");
      if (!isArchiveDrop(event.payload.paths)) {
        setUpdateMessage("Drop only ZIP, 7Z, or RAR chart archives.");
        return;
      }
      void inspectArchives(event.payload.paths);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [inspectArchives]);

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
    const timeout = window.setTimeout(() => {
      setPackPage(0);
      setPackQuery(packQueryInput.trim());
    }, 280);
    return () => window.clearTimeout(timeout);
  }, [packQueryInput]);

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

  const loadPacks = useCallback(async () => {
    if (view !== "packs") return;
    setPacksLoading(true);
    setPackError("");
    try {
      const result = await fetchPacks(packQuery, packPage);
      const normalized = {
        ...result,
        count: Number.isFinite(result.count) ? result.count : result.packs.length,
        nextPage: result.nextPage ?? null,
        packs: result.packs.map((pack) => ({ ...pack, charts: pack.charts ?? [] }))
      };
      setPackCatalog(normalized);
      setPackSelections((current) => {
        const next = { ...current };
        for (const pack of normalized.packs) {
          if (!next[pack.id]) next[pack.id] = new Set(pack.charts.map((chart) => chart.id));
        }
        return next;
      });
    } catch (error) {
      setPackError(error instanceof Error ? error.message : "Could not load packs.");
    } finally {
      setPacksLoading(false);
    }
  }, [packPage, packQuery, view]);

  useEffect(() => {
    void loadPacks();
  }, [loadPacks]);

  const refreshLibrary = useCallback(async () => {
    if (!isTauri() || !targetDirectory) return;
    setLibraryLoading(true);
    try {
      const [installed, trash, savedBackups] = await Promise.all([
        invoke<InstalledChart[]>("list_installed_charts", { path: targetDirectory }),
        invoke<TrashItem[]>("list_trashed_charts", { path: targetDirectory }),
        invoke<BackupItem[]>("list_chart_backups", { path: targetDirectory })
      ]);
      setInstalledCharts(installed);
      setTrashItems(trash);
      setBackups(savedBackups);
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
  const installedIds = useMemo(() => {
    const ids = new Set(installedCharts.flatMap((chart) => chart.chartId ? [chart.chartId] : []));
    for (const match of manualMatches) ids.add(match.chart.id);
    for (const chart of catalog.charts) {
      if (installedCharts.some((installed) => chartMatchesInstalledChart(chart, installed))) {
        ids.add(chart.id);
      }
    }
    return ids;
  }, [catalog.charts, installedCharts, manualMatches]);
  const manualMatchesByPath = useMemo(
    () => new Map(manualMatches.map((match) => [match.installedPath, match])),
    [manualMatches]
  );
  const canEnableAllUpdates = manualMatches.length > 0 ||
    installedCharts.some((chart) => chart.managed && !chart.updatesEnabled);
  const visibleInstalledCharts = useMemo(() => {
    const needle = installedQuery.trim().toLocaleLowerCase();
    return installedCharts.filter((chart) => {
      return !needle || [chart.title, chart.artist, chart.charter, chart.folderName]
        .filter(Boolean)
        .some((value) => value!.toLocaleLowerCase().includes(needle));
    });
  }, [installedCharts, installedQuery]);
  const pagedInstalledCharts = visibleInstalledCharts.slice(0, installedVisibleLimit);

  useEffect(() => {
    setInstalledVisibleLimit(40);
  }, [installedQuery]);

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
    if (!isTauri() || !appVisible || !automaticUpdates || !appState?.directoryExists || !targetDirectory) return;
    let cancelled = false;
    const checkForUpdates = async () => {
      try {
        const updates = await invoke<UpdateCandidate[]>("check_installed_updates", { path: targetDirectory });
        if (cancelled) return;
        setUpdateCandidates(updates);
        if (updates.length === 0) return;
        let completed = 0;
        for (const update of updates) {
          if (cancelled) return;
          if (await install(update.chart)) completed += 1;
        }
        if (!cancelled && completed === updates.length) setUpdateMessage("");
        if (!cancelled) setUpdateCandidates([]);
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
  }, [appState?.directoryExists, appVisible, automaticUpdates, targetDirectory]);

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

  async function enableAllChartUpdates() {
    if (!targetDirectory || bulkUpdatesLoading) return;
    setBulkUpdatesLoading(true);
    setUpdateMessage("");
    try {
      let adopted = 0;
      let failed = 0;
      for (let index = 0; index < manualMatches.length; index += matchAdoptionBatchSize) {
        const batch = manualMatches.slice(index, index + matchAdoptionBatchSize);
        const results = await Promise.allSettled(batch.map((match) =>
          invoke("adopt_manual_chart", {
            chartId: match.chart.id,
            installedPath: match.installedPath,
            path: targetDirectory
          })
        ));
        adopted += results.filter((result) => result.status === "fulfilled").length;
        failed += results.filter((result) => result.status === "rejected").length;
      }
      const managed = await invoke<number>("set_all_chart_updates", {
        enabled: true,
        path: targetDirectory
      });
      setAutomaticUpdates(true);
      localStorage.setItem("unchartable:auto-updates", "on");
      setUpdateMessage(
        `${managed} chart update${managed === 1 ? "" : "s"} enabled` +
        `${adopted > 0 ? `, including ${adopted} newly linked local chart${adopted === 1 ? "" : "s"}` : ""}` +
        `${failed > 0 ? `. ${failed} local chart${failed === 1 ? "" : "s"} could not be linked` : ""}.`
      );
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not enable every chart update: ${String(error)}`);
    } finally {
      setBulkUpdatesLoading(false);
    }
  }

  async function checkUpdates(showMessage = true) {
    if (!targetDirectory || updatesLoading) return [];
    setUpdatesLoading(true);
    try {
      const updates = await invoke<UpdateCandidate[]>("check_installed_updates", { path: targetDirectory });
      setUpdateCandidates(updates);
      if (showMessage) {
        setUpdateMessage(updates.length
          ? `${updates.length} chart update${updates.length === 1 ? "" : "s"} available.`
          : "Your managed charts are up to date.");
      }
      return updates;
    } catch (error) {
      if (showMessage) setUpdateMessage(`Could not check chart updates: ${String(error)}`);
      return [];
    } finally {
      setUpdatesLoading(false);
    }
  }

  async function installUpdates(candidates: UpdateCandidate[]) {
    let completed = 0;
    for (const candidate of candidates) {
      if (await install(candidate.chart)) completed += 1;
    }
    await checkUpdates(false);
    setUpdateMessage(`${completed} of ${candidates.length} update${candidates.length === 1 ? "" : "s"} installed.`);
  }

  async function installPack(pack: ChartPack) {
    let completePack: ChartPack;
    try {
      completePack = await fetchPack(pack.id);
    } catch (error) {
      setUpdateMessage(`Could not load the full pack: ${String(error)}`);
      return;
    }
    const visibleChartIds = new Set(pack.charts.map((chart) => chart.id));
    const visibleSelection = packSelections[pack.id] ?? visibleChartIds;
    const charts = completePack.charts.filter((chart) => {
      const selected = !visibleChartIds.has(chart.id) || visibleSelection.has(chart.id);
      return selected && !installedIds.has(chart.id);
    });
    if (charts.length === 0) {
      setUpdateMessage("Every selected chart from this pack is already installed.");
      return;
    }
    setView("downloads");
    let completed = 0;
    for (const chart of charts) {
      if (await install(chart)) completed += 1;
    }
    setUpdateMessage(`${completed} of ${charts.length} selected pack chart${charts.length === 1 ? "" : "s"} installed.`);
  }

  function togglePackChart(packId: string, chartId: string) {
    setPackSelections((current) => {
      const next = new Set(current[packId] ?? []);
      if (next.has(chartId)) next.delete(chartId);
      else next.add(chartId);
      return { ...current, [packId]: next };
    });
  }

  async function restoreBackup(item: BackupItem) {
    if (!targetDirectory || !window.confirm(`Restore the previous version of "${item.title}"? The current version will become a backup.`)) return;
    try {
      await invoke("restore_chart_backup", { backupId: item.backupId, path: targetDirectory });
      setUpdateMessage(`${item.title} restored to its previous version.`);
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not restore ${item.title}: ${String(error)}`);
    }
  }

  async function deleteBackup(item: BackupItem) {
    if (!targetDirectory || !window.confirm(`Permanently delete this backup of "${item.title}"?`)) return;
    try {
      await invoke("delete_chart_backup", { backupId: item.backupId, path: targetDirectory });
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not delete this backup: ${String(error)}`);
    }
  }

  async function runDiagnostics() {
    if (!targetDirectory) return;
    setDiagnosticLoading(true);
    setUpdateMessage("");
    try {
      setDiagnostic(await invoke<DiagnosticReport>("diagnose_library", { path: targetDirectory }));
    } catch (error) {
      setUpdateMessage(`Diagnostics failed: ${String(error)}`);
    } finally {
      setDiagnosticLoading(false);
    }
  }

  async function chooseChartArchive() {
    if (!isTauri() || !targetDirectory || importLoading) return;
    const selected = await open({
      directory: false,
      filters: [{ name: "Chart archive", extensions: ["zip", "7z", "rar"] }],
      multiple: true,
      title: "Import chart archives"
    });
    if (!selected) return;
    await inspectArchives(Array.isArray(selected) ? selected : [selected]);
  }

  async function confirmChartImports() {
    if (!importInspections.length || !targetDirectory) return;
    const singleConflict = importInspections.length === 1 && Boolean(importInspections[0].conflictPath);
    const pending = singleConflict
      ? importInspections
      : importInspections.filter((inspection) => !inspection.conflictPath);
    if (!pending.length) {
      setUpdateMessage("Every selected chart is already installed.");
      return;
    }
    setImportLoading(true);
    let imported = 0;
    let failed = 0;
    try {
      for (const inspection of pending) {
        try {
          await invoke("import_chart_archive", {
            allowDuplicate: singleConflict,
            archivePath: inspection.archivePath,
            targetDirectory
          });
          imported += 1;
        } catch {
          failed += 1;
        }
      }
      const skipped = importInspections.length - pending.length;
      setUpdateMessage([
        `${imported} chart${imported === 1 ? "" : "s"} imported.`,
        skipped ? `${skipped} already installed.` : "",
        failed ? `${failed} failed.` : ""
      ].filter(Boolean).join(" "));
      setImportInspections([]);
      await refreshLibrary();
    } finally {
      setImportLoading(false);
    }
  }

  async function repairLocalLibrary() {
    if (!targetDirectory || repairLoading) return;
    setRepairLoading(true);
    try {
      const report = await invoke<RepairReport>("repair_library", { path: targetDirectory });
      setRepairReport(report);
      setUpdateMessage(
        `Library repair removed ${report.removedTemporaryItems} temporary item${report.removedTemporaryItems === 1 ? "" : "s"} and found ${report.invalidChartPaths.length} incomplete chart folder${report.invalidChartPaths.length === 1 ? "" : "s"}.`
      );
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Library repair failed: ${String(error)}`);
    } finally {
      setRepairLoading(false);
    }
  }

  function createLocalPack() {
    const chartPaths = installedCharts
      .filter((chart) => selectedInstalled.has(chart.path) && chart.playable)
      .map((chart) => chart.path);
    if (!chartPaths.length) return;
    const name = window.prompt("Name this local pack:");
    if (!name?.trim()) return;
    setLocalPacks((current) => [{
      chartPaths,
      createdAt: new Date().toISOString(),
      id: crypto.randomUUID(),
      name: name.trim()
    }, ...current]);
    setSelectedInstalled(new Set());
    setUpdateMessage(`${name.trim()} saved as a local pack.`);
  }

  async function exportLocalPack(pack: LocalPack) {
    if (!targetDirectory) return;
    const output = await save({
      defaultPath: `${pack.name.replace(/[<>:"/\\|?*]+/g, "").trim() || "UNCHARTABLE Pack"}.zip`,
      filters: [{ name: "ZIP archive", extensions: ["zip"] }],
      title: "Export local chart pack"
    });
    if (!output) return;
    try {
      await invoke("export_local_pack", {
        chartPaths: pack.chartPaths,
        name: pack.name,
        outputPath: output,
        path: targetDirectory
      });
      setUpdateMessage(`${pack.name} exported successfully.`);
      await refreshLibrary();
    } catch (error) {
      setUpdateMessage(`Could not export ${pack.name}: ${String(error)}`);
    }
  }

  function toggleInstalledSelection(path: string) {
    setSelectedInstalled((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  async function enableSelectedUpdates() {
    const selected = installedCharts.filter((chart) => selectedInstalled.has(chart.path));
    for (const chart of selected) {
      const match = manualMatchesByPath.get(chart.path);
      if (chart.managed && chart.chartId) await setChartUpdates(chart, true);
      else if (match) await adoptManualChart(chart, match);
    }
    setSelectedInstalled(new Set());
    await refreshLibrary();
  }

  async function removeSelectedCharts() {
    const selected = installedCharts.filter((chart) => selectedInstalled.has(chart.path) && chart.managed && chart.chartId);
    if (!selected.length || !window.confirm(`Move ${selected.length} selected chart${selected.length === 1 ? "" : "s"} to trash?`)) return;
    let removed = 0;
    for (const chart of selected) {
      try {
        await invoke("trash_installed_chart", { chartId: chart.chartId, path: targetDirectory });
        removed += 1;
      } catch {
        // Continue so one damaged chart does not block the batch.
      }
    }
    setSelectedInstalled(new Set());
    setUpdateMessage(`${removed} selected chart${removed === 1 ? "" : "s"} moved to trash.`);
    await refreshLibrary();
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

  async function resumePausedDownloads() {
    const paused = Object.values(installing).filter((entry) => Boolean(entry.error));
    for (const entry of paused) await install(entry.chart);
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
          <NavButton active={view === "packs"} icon={<Package />} label="packs" onClick={() => setView("packs")} />
          <NavButton
            active={view === "downloads"}
            badge={activeInstalls || undefined}
            icon={<Download />}
            label="downloads"
            onClick={() => setView("downloads")}
          />
          <NavButton
            active={view === "updates"}
            badge={updateCandidates.length || undefined}
            icon={<RefreshCw />}
            label="updates"
            onClick={() => {
              setView("updates");
              void checkUpdates(false);
            }}
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

        {view === "packs" ? (
          <>
            <header className="page-header">
              <div><p className="kicker">community collections</p><h1>chart packs</h1></div>
              <div className="catalog-total">{packCatalog.count} packs</div>
            </header>
            <section className="filter-bar pack-filter">
              <label className="search-field">
                <Search />
                <input
                  aria-label="Search packs"
                  onChange={(event) => setPackQueryInput(event.target.value)}
                  placeholder="search pack name or creator"
                  value={packQueryInput}
                />
                {packQueryInput ? <button aria-label="Clear search" onClick={() => setPackQueryInput("")} type="button"><X /></button> : null}
              </label>
            </section>
            {packError ? (
              <section className="empty-state"><CircleAlert /><h2>could not load packs</h2><p>{packError}</p><button onClick={() => void loadPacks()} type="button">try again</button></section>
            ) : packsLoading ? (
              <section className="loading-state"><LoaderCircle /><span>loading packs</span></section>
            ) : packCatalog.packs.length === 0 ? (
              <section className="empty-state"><Package /><h2>no packs found</h2><p>Try a different search.</p></section>
            ) : (
              <section className="pack-grid">
                {packCatalog.packs.map((pack) => {
                  const selected = packSelections[pack.id] ?? new Set<string>();
                  const remaining = pack.charts.filter((chart) => selected.has(chart.id) && !installedIds.has(chart.id)).length;
                  return (
                    <article className="pack-card" key={pack.id}>
                      <div className="pack-card-head">
                        <div>
                          <p className="kicker">{pack.owner?.displayName || "community pack"}</p>
                          <h2>{pack.name}</h2>
                          <p>{pack.description || `${pack.chartCount} charts ready to install.`}</p>
                        </div>
                        <span>{pack.chartCount}</span>
                      </div>
                      <div className="pack-chart-list">
                        {pack.charts.map((chart) => {
                          const installed = installedIds.has(chart.id);
                          const checked = selected.has(chart.id);
                          return (
                            <button
                              className={installed ? "pack-chart pack-chart-installed" : "pack-chart"}
                              disabled={installed}
                              key={chart.id}
                              onClick={() => togglePackChart(pack.id, chart.id)}
                              type="button"
                            >
                              {installed || checked ? <CheckSquare /> : <Square />}
                              <img alt="" loading="lazy" src={buildChartCoverUrl(chart)} />
                              <span><strong>{chart.title}</strong><small>{installed ? "installed" : `${chart.artist} · ${chart.charterName}`}</small></span>
                            </button>
                          );
                        })}
                      </div>
                      <button className="pack-install-button" disabled={remaining === 0} onClick={() => void installPack(pack)} type="button">
                        <Package /> {remaining === 0 ? "selected charts installed" : `install ${remaining} selected`}
                      </button>
                    </article>
                  );
                })}
              </section>
            )}
            <footer className="pagination">
              <button disabled={packPage === 0 || packsLoading} onClick={() => setPackPage((current) => Math.max(0, current - 1))} type="button"><ChevronLeft /> previous</button>
              <span>page {packPage + 1}</span>
              <button disabled={packCatalog.nextPage === null || packsLoading} onClick={() => setPackPage(packCatalog.nextPage ?? packPage)} type="button">next <ChevronRight /></button>
            </footer>
          </>
        ) : null}

        {view === "updates" ? (
          <>
            <header className="page-header">
              <div><p className="kicker">managed library</p><h1>updates</h1></div>
              <div className="download-header-actions">
                <button disabled={updatesLoading} onClick={() => void checkUpdates()} type="button"><RefreshCw className={updatesLoading ? "spin" : ""} /> check now</button>
                <button disabled={!updateCandidates.length || updatesLoading} onClick={() => void installUpdates(updateCandidates)} type="button"><Download /> update all</button>
              </div>
            </header>
            {updateMessage ? <div className="status-banner"><PackageCheck /> {updateMessage}</div> : null}
            {updatesLoading && updateCandidates.length === 0 ? (
              <section className="loading-state"><LoaderCircle /><span>checking installed versions</span></section>
            ) : updateCandidates.length === 0 ? (
              <section className="empty-state"><PackageCheck /><h2>everything is current</h2><p>Managed charts will be checked again automatically.</p></section>
            ) : (
              <section className="update-list">
                {updateCandidates.map((candidate) => (
                  <article className="update-row" key={candidate.chart.id}>
                    <img alt="" src={buildChartCoverUrl(candidate.chart)} />
                    <div>
                      <h2>{candidate.chart.title}</h2>
                      <p>{candidate.chart.artist} · charted by {candidate.chart.charterName}</p>
                      <small>installed {candidate.installedVersion ? new Date(candidate.installedVersion).toLocaleString() : "unknown"} · published {new Date(candidate.latestVersion).toLocaleString()}</small>
                    </div>
                    <button onClick={() => void installUpdates([candidate])} type="button"><Download /> update</button>
                  </article>
                ))}
              </section>
            )}
            {backups.length > 0 ? (
              <section className="library-section">
                <div className="section-heading"><div><p className="kicker">rollback</p><h2>previous versions</h2></div><span>{backups.length}</span></div>
                <div className="backup-list">
                  {backups.map((item) => (
                    <article className="backup-row" key={item.backupId}>
                      <History />
                      <div><strong>{item.title}</strong><small>{formatBytes(item.sizeBytes)} · saved {new Date(item.createdAt).toLocaleString()}</small></div>
                      <button onClick={() => void restoreBackup(item)} title="restore this version" type="button"><Undo2 /> restore</button>
                      <button className="danger-icon" onClick={() => void deleteBackup(item)} title="delete backup" type="button"><Trash2 /></button>
                    </article>
                  ))}
                </div>
              </section>
            ) : null}
          </>
        ) : null}

        {view === "downloads" ? (
          <>
            <header className="page-header">
              <div><p className="kicker">downloads and CustomSongs</p><h1>your charts</h1></div>
              <div className="download-header-actions">
                <span className="catalog-total">{activeInstalls} active</span>
                <button disabled={importLoading} onClick={() => void chooseChartArchive()} type="button">
                  <Upload /> {importLoading ? "checking archive" : "import archive"}
                </button>
                <button disabled={repairLoading} onClick={() => void repairLocalLibrary()} type="button">
                  <Hammer className={repairLoading ? "spin" : ""} /> repair
                </button>
                <button disabled={libraryLoading} onClick={() => void refreshLibrary()} type="button">
                  <RefreshCw className={libraryLoading ? "spin" : ""} /> refresh
                </button>
                {Object.values(installing).some((entry) => entry.error) ? (
                  <button onClick={() => void resumePausedDownloads()} type="button"><Play /> resume all</button>
                ) : null}
                {Object.values(installing).some((entry) => entry.progress.stage === "complete" || entry.error) ? (
                  <button onClick={clearFinishedDownloads} type="button"><Trash2 /> clear finished</button>
                ) : null}
              </div>
            </header>
            {updateMessage ? <div className="status-banner"><PackageCheck /> {updateMessage}</div> : null}
            {importInspections.length ? (
              <section className={importInspections.every((inspection) => inspection.conflictPath) ? "import-review import-review-conflict" : "import-review import-review-ready"}>
                <div className="import-review-icon" aria-hidden="true"><Archive /></div>
                <div className="import-review-copy">
                  <div className="import-review-heading">
                    <p className="kicker">{importInspections.length === 1 ? "archive checked" : `${importInspections.length} archives checked`}</p>
                  </div>
                  <div className="import-review-list">
                    {importInspections.map((inspection, index) => (
                      <article className={inspection.conflictPath ? "import-review-item import-review-item-conflict" : "import-review-item"} key={`${inspection.archivePath}-${index}`}>
                        <div>
                          <h2>{inspection.title}</h2>
                          <div className="import-review-meta">
                            <span>{inspection.artist}</span>
                            <span>charted by {inspection.charter}</span>
                            <span>{formatBytes(inspection.archiveSizeBytes)}</span>
                          </div>
                        </div>
                        <span className="archive-format">{inspection.archiveFormat}</span>
                        {inspection.conflictPath
                          ? <CircleAlert aria-label="already installed" />
                          : <ShieldCheck aria-label="ready to install" />}
                      </article>
                    ))}
                  </div>
                </div>
                <div className="import-review-actions">
                  <button className="import-confirm-button" disabled={importLoading} onClick={() => void confirmChartImports()} type="button">
                    <Upload /> {
                      importInspections.length === 1 && importInspections[0].conflictPath
                        ? "install another copy"
                        : `install ${importInspections.filter((inspection) => !inspection.conflictPath).length}`
                    }
                  </button>
                  <button className="import-cancel-button" onClick={() => setImportInspections([])} type="button"><X /> cancel</button>
                </div>
              </section>
            ) : null}
            {repairReport?.invalidChartPaths.length ? (
              <section className="repair-report">
                <CircleAlert />
                <div>
                  <strong>{repairReport.invalidChartPaths.length} incomplete chart folder{repairReport.invalidChartPaths.length === 1 ? "" : "s"} need manual attention.</strong>
                  <small>{repairReport.invalidChartPaths.slice(0, 3).join(" / ")}</small>
                </div>
                <button onClick={() => void openTargetDirectory()} type="button"><FolderOpen /> open folder</button>
              </section>
            ) : null}
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
                  <button
                    className="enable-all-updates"
                    disabled={bulkUpdatesLoading || !canEnableAllUpdates}
                    onClick={() => void enableAllChartUpdates()}
                    title="link recognized local charts and enable all automatic updates"
                    type="button"
                  >
                    <RefreshCw className={bulkUpdatesLoading ? "spin" : ""} />
                    {bulkUpdatesLoading ? "enabling" : canEnableAllUpdates ? "enable all updates" : "updates enabled"}
                  </button>
                  <span>{visibleInstalledCharts.length}</span>
                </div>
              </div>
              {selectedInstalled.size > 0 ? (
                <div className="library-toolbar">
                  <div className="batch-actions">
                    <span>{selectedInstalled.size} selected</span>
                    <button onClick={() => void enableSelectedUpdates()} type="button"><RefreshCw /> enable updates</button>
                    <button onClick={createLocalPack} type="button"><Package /> create pack</button>
                    <button className="danger-button" onClick={() => void removeSelectedCharts()} type="button"><Trash2 /> trash</button>
                    <button onClick={() => setSelectedInstalled(new Set())} type="button"><X /> clear</button>
                  </div>
                </div>
              ) : null}
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
                {pagedInstalledCharts.map((chart) => {
                  const manualMatch = manualMatchesByPath.get(chart.path);
                  return (
                  <article
                    className={selectedInstalled.has(chart.path) ? "installed-row installed-row-selected" : "installed-row"}
                    key={chart.path}
                    onClick={(event) => {
                      if ((event.target as HTMLElement).closest("button")) return;
                      toggleInstalledSelection(chart.path);
                    }}
                  >
                    <button
                      aria-label={`${selectedInstalled.has(chart.path) ? "Deselect" : "Select"} ${chart.title}`}
                      className="selection-button"
                      onClick={() => toggleInstalledSelection(chart.path)}
                      type="button"
                    >
                      {selectedInstalled.has(chart.path) ? <CheckSquare /> : <Square />}
                    </button>
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
                      <small className={`identity-status ${!chart.playable ? "identity-problem" : chart.managed ? "identity-managed" : manualMatch ? "identity-match" : "identity-manual"}`}>
                        {!chart.playable
                          ? "problem: chart text or supported audio is missing"
                          : chart.managed
                            ? "managed: linked by its UNCHARTABLE chart ID"
                            : manualMatch
                              ? "recognized: title, artist, charter, and duration match"
                              : "manual: no unique safe match was found"}
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
            {visibleInstalledCharts.length > installedVisibleLimit ? (
              <button className="load-more-button" onClick={() => setInstalledVisibleLimit((current) => current + 40)} type="button">
                show more charts
              </button>
            ) : null}
            </section>
            {localPacks.length > 0 ? (
              <section className="library-section">
                <div className="section-heading"><div><p className="kicker">your collections</p><h2>local packs</h2></div><span>{localPacks.length}</span></div>
                <div className="local-pack-list">
                  {localPacks.map((pack) => (
                    <article className="local-pack-row" key={pack.id}>
                      <Package />
                      <div><strong>{pack.name}</strong><small>{pack.chartPaths.length} charts / created {new Date(pack.createdAt).toLocaleDateString()}</small></div>
                      <button onClick={() => void exportLocalPack(pack)} type="button"><Archive /> export ZIP</button>
                      <button className="danger-icon" onClick={() => setLocalPacks((current) => current.filter((item) => item.id !== pack.id))} title="delete local pack" type="button"><Trash2 /></button>
                    </article>
                  ))}
                </div>
              </section>
            ) : null}
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
              <div className="settings-divider" />
              <div className="diagnostic-heading">
                <div className="setting-copy">
                  <Wrench />
                  <div>
                    <h2>library diagnostics</h2>
                    <p>Review chart integrity, library size, trash, and rollback storage.</p>
                  </div>
                </div>
                <button disabled={diagnosticLoading || !targetDirectory} onClick={() => void runDiagnostics()} type="button">
                  <Wrench className={diagnosticLoading ? "spin" : ""} /> run diagnostics
                </button>
              </div>
              {diagnostic ? (
                <div className="diagnostic-grid">
                  <DiagnosticStat label="installed" value={`${diagnostic.totalCharts} charts`} />
                  <DiagnosticStat label="managed / manual" value={`${diagnostic.managedCharts} / ${diagnostic.manualCharts}`} />
                  <DiagnosticStat label="problems" value={String(diagnostic.invalidCharts)} warning={diagnostic.invalidCharts > 0} />
                  <DiagnosticStat label="library size" value={formatBytes(diagnostic.totalSizeBytes)} />
                  <DiagnosticStat label="trash" value={`${diagnostic.trashCount} · ${formatBytes(diagnostic.trashSizeBytes)}`} />
                  <DiagnosticStat label="backups" value={`${diagnostic.backupCount} · ${formatBytes(diagnostic.backupSizeBytes)}`} />
                </div>
              ) : null}
              <div className="settings-divider" />
              <div className="support-row">
                <div className="setting-copy">
                  <Bug />
                  <div>
                    <h2>report a bug</h2>
                    <p>Open a GitHub report with the app version and operating system already included.</p>
                  </div>
                </div>
                <button
                  onClick={() => void openExternalUrl(buildBugReportUrl(appPackage.version, navigator.userAgent))}
                  type="button"
                >
                  <Bug /> report bug
                </button>
              </div>
              <p className="app-version">UNCHARTABLE v{appPackage.version}</p>
            </section>
          </>
        ) : null}
      </main>
      {dropState ? (
        <div className={dropState === "valid" ? "drop-overlay drop-overlay-valid" : "drop-overlay drop-overlay-invalid"}>
          <div>
            {dropState === "valid" ? <Upload /> : <CircleAlert />}
            <strong>{dropState === "valid" ? "drop to import charts" : "unsupported files"}</strong>
            <span>{dropState === "valid" ? "ZIP, 7Z, and RAR archives are checked before installation" : "drop only ZIP, 7Z, or RAR chart archives"}</span>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function DiagnosticStat({ label, value, warning = false }: { label: string; value: string; warning?: boolean }) {
  return (
    <div className={warning ? "diagnostic-stat diagnostic-stat-warning" : "diagnostic-stat"}>
      {warning ? <CircleAlert /> : <Check />}
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function NavButton({ active, badge, icon, label, onClick }: { active: boolean; badge?: number; icon: React.ReactNode; label: string; onClick: () => void }) {
  return (
    <button aria-current={active ? "page" : undefined} aria-label={label} className={active ? "nav-button nav-button-active" : "nav-button"} onClick={onClick} title={label} type="button">
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
