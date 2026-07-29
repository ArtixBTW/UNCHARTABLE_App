export const API_ORIGIN = "https://unchartable.site";

export function isSingleZipDrop(paths: string[]) {
  return paths.length === 1 && paths[0].toLowerCase().endsWith(".zip");
}

export type DifficultyLevel = {
  charterName?: string | null;
  difficulty: string;
  level: number;
};

export type Chart = {
  artist: string;
  audioDurationSeconds: number | null;
  charterName: string;
  difficulty: string;
  difficultyLevel: number;
  difficultyLevels: DifficultyLevel[];
  downloadCount: number;
  hasDirectDownload: boolean;
  id: string;
  rankedStatus: string;
  songId: string;
  submitter?: {
    displayName?: string | null;
    discordUsername?: string | null;
  } | null;
  title: string;
  contentUpdatedAt?: string | null;
  updatedAt: string;
};

export type ChartCatalog = {
  charts: Chart[];
  count: number;
  nextPage: number | null;
};

export type ChartPack = {
  chartCount: number;
  charts: Chart[];
  description: string;
  id: string;
  name: string;
  owner?: {
    displayName?: string | null;
  } | null;
  updatedAt: string;
};

export type PackCatalog = {
  count: number;
  nextPage: number | null;
  packs: ChartPack[];
};

export type AppState = {
  customSongsPath: string;
  directoryExists: boolean;
};

export type InstallProgress = {
  chartId: string;
  downloadedBytes: number;
  totalBytes: number | null;
  stage: "queued" | "requesting" | "downloading" | "installing" | "complete" | "failed";
};

export type InstallResult = {
  archiveSha256: string;
  chartId: string;
  installPath: string;
};

export type InstalledChart = {
  audioDurationSeconds: number | null;
  artist: string | null;
  chartId: string | null;
  charter: string | null;
  folderName: string;
  installedAt: string | null;
  managed: boolean;
  path: string;
  playable: boolean;
  sizeBytes: number;
  title: string;
  updatedAt: string | null;
  updatesEnabled: boolean;
};

export type ManualChartMatch = {
  chart: Chart;
  installedPath: string;
};

export type TrashItem = {
  chartId: string | null;
  deletedAt: string;
  originalFolderName: string;
  sizeBytes: number;
  title: string;
  trashId: string;
};

export type UpdateCandidate = {
  chart: Chart;
  installedVersion: string | null;
  latestVersion: string;
};

export type BackupItem = {
  backupId: string;
  chartId: string;
  createdAt: string;
  folderName: string;
  sizeBytes: number;
  title: string;
};

export type DiagnosticReport = {
  backupCount: number;
  backupSizeBytes: number;
  freeSpaceBytes: number;
  invalidCharts: number;
  managedCharts: number;
  manualCharts: number;
  path: string;
  pathWritable: boolean;
  totalCharts: number;
  totalSizeBytes: number;
  trashCount: number;
  trashSizeBytes: number;
};

export type ImportArchiveInspection = {
  archivePath: string;
  archiveSizeBytes: number;
  artist: string;
  charter: string;
  conflictPath: string | null;
  title: string;
};

export type RepairReport = {
  invalidChartPaths: string[];
  removedTemporaryItems: number;
};

export type OperationRecord = {
  action: string;
  createdAt: string;
  detail: string;
  title: string;
};

export type LocalPack = {
  chartPaths: string[];
  createdAt: string;
  id: string;
  name: string;
};

export type LibraryFilter = "all" | "updates" | "managed" | "manual" | "problems";

export function chartContentVersion(chart: Pick<Chart, "contentUpdatedAt" | "updatedAt">) {
  return chart.contentUpdatedAt || chart.updatedAt;
}

export function buildChartCoverUrl(chart: Pick<Chart, "id" | "updatedAt">) {
  return `${API_ORIGIN}/api/charts/${encodeURIComponent(chart.id)}/cover?size=thumb&v=${encodeURIComponent(chart.updatedAt)}`;
}

export function buildChartPublicUrl(chart: Pick<Chart, "id">) {
  return `${API_ORIGIN}/charts/${encodeURIComponent(chart.id)}`;
}

export function buildChartPreviewUrl(chart: Pick<Chart, "id">) {
  return `${API_ORIGIN}/api/charts/${encodeURIComponent(chart.id)}/preview`;
}

export function parseInstallDeepLink(value: string) {
  try {
    const url = new URL(value);
    if (url.protocol !== "unchartable:" || url.hostname !== "install") return null;
    const chartId = url.pathname.split("/").filter(Boolean)[0] ?? "";
    return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(chartId)
      ? chartId
      : null;
  } catch {
    return null;
  }
}

export function normalizeChartIdentity(value: string | null | undefined) {
  return (value ?? "")
    .normalize("NFKD")
    .replace(/\p{Diacritic}/gu, "")
    .toLocaleLowerCase()
    .replace(/[^\p{Letter}\p{Number}]+/gu, "");
}

export function chartMatchesInstalledChart(chart: Chart, installed: InstalledChart) {
  if (installed.chartId) return installed.chartId === chart.id;
  if (!installed.playable) return false;

  const titleMatches = normalizeChartIdentity(installed.title) === normalizeChartIdentity(chart.title);
  const artistMatches = normalizeChartIdentity(installed.artist) === normalizeChartIdentity(chart.artist);
  const localCharter = normalizeChartIdentity(installed.charter);
  if (!titleMatches || !artistMatches || !localCharter) return false;

  const siteCharters = new Set(
    [
      chart.charterName,
      chart.submitter?.displayName,
      chart.submitter?.discordUsername,
      ...chart.difficultyLevels.map((level) => level.charterName)
    ]
      .map(normalizeChartIdentity)
      .filter(Boolean)
  );
  if (!siteCharters.has(localCharter)) return false;

  return installed.audioDurationSeconds === null ||
    chart.audioDurationSeconds === null ||
    Math.abs(installed.audioDurationSeconds - chart.audioDurationSeconds) <= 3;
}

export function difficultyClass(difficulty: string) {
  const normalized = difficulty.toLowerCase();
  return `difficulty-${["beginner", "normal", "hard", "expert", "unbeatable", "star"].includes(normalized) ? normalized : "normal"}`;
}

export function formatDuration(seconds: number | null) {
  if (!seconds || seconds < 1) return "duration unknown";
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(Math.round(seconds % 60)).padStart(2, "0")}`;
}

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
