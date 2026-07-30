import { describe, expect, it } from "vitest";
import {
  buildChartCoverUrl,
  buildChartPreviewUrl,
  buildChartPublicUrl,
  chartContentVersion,
  chartMatchesInstalledChart,
  difficultyClass,
  formatBytes,
  formatDuration,
  isArchiveDrop,
  parseInstallDeepLink,
  type Chart,
  type InstalledChart
} from "./lib";

const chart: Chart = {
  artist: "Vane Lily",
  audioDurationSeconds: 208.9,
  charterName: "firelordethan",
  contentUpdatedAt: null,
  difficulty: "STAR",
  difficultyLevel: 21,
  difficultyLevels: [{ charterName: "firelordethan", difficulty: "STAR", level: 21 }],
  downloadCount: 1,
  hasDirectDownload: true,
  id: "0aaf5a3a-f9c6-4fbe-9306-b5010a85d6dd",
  rankedStatus: "unranked",
  songId: "harlequin",
  submitter: { displayName: "firelordethan", discordUsername: "firelordethan" },
  title: "Harlequin Contraption",
  updatedAt: "2026-07-23T12:00:00Z"
};

const installed: InstalledChart = {
  artist: "vane lily",
  audioDurationSeconds: 208.968,
  chartId: null,
  charter: "FireLordEthan",
  folderName: "harlequin contraption",
  installedAt: null,
  managed: false,
  path: "C:\\CustomSongs\\harlequin contraption",
  playable: true,
  sizeBytes: 1,
  title: "harlequin contraption",
  updatedAt: null,
  updatesEnabled: false
};

describe("chart presentation helpers", () => {
  it("builds a versioned cover URL", () => {
    expect(buildChartCoverUrl({ id: "chart id", updatedAt: "2026-07-23T12:00:00Z" }))
      .toBe("https://unchartable.site/api/charts/chart%20id/cover?size=thumb&v=2026-07-23T12%3A00%3A00Z");
  });

  it("builds the public chart URL from its stable song id", () => {
    expect(buildChartPublicUrl({ id: "0aaf5a3a-f9c6-4fbe-9306-b5010a85d6dd" }))
      .toBe("https://unchartable.site/charts/0aaf5a3a-f9c6-4fbe-9306-b5010a85d6dd");
  });

  it("builds the protected preview URL", () => {
    expect(buildChartPreviewUrl({ id: "chart id" }))
      .toBe("https://unchartable.site/api/charts/chart%20id/preview");
  });

  it("keeps official difficulty classes stable", () => {
    expect(difficultyClass("STAR")).toBe("difficulty-star");
    expect(difficultyClass("UNBEATABLE")).toBe("difficulty-unbeatable");
  });

  it("formats duration and download progress", () => {
    expect(formatDuration(189)).toBe("3:09");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
  });

  it("prefers the content version for automatic updates", () => {
    expect(chartContentVersion({
      contentUpdatedAt: "2026-07-23T13:00:00Z",
      updatedAt: "2026-07-23T14:00:00Z"
    })).toBe("2026-07-23T13:00:00Z");
  });

  it("accepts only install deep links with a UUID chart id", () => {
    expect(parseInstallDeepLink("unchartable://install/0aaf5a3a-f9c6-4fbe-9306-b5010a85d6dd"))
      .toBe("0aaf5a3a-f9c6-4fbe-9306-b5010a85d6dd");
    expect(parseInstallDeepLink("unchartable://open/0aaf5a3a-f9c6-4fbe-9306-b5010a85d6dd")).toBeNull();
    expect(parseInstallDeepLink("https://unchartable.site/charts/0aaf5a3a-f9c6-4fbe-9306-b5010a85d6dd")).toBeNull();
    expect(parseInstallDeepLink("unchartable://install/not-a-chart")).toBeNull();
  });

  it("accepts exactly one supported chart archive", () => {
    expect(isArchiveDrop(["C:\\Downloads\\chart.ZIP"])).toBe(true);
    expect(isArchiveDrop(["C:\\Downloads\\song.name.with.dots.7z"])).toBe(true);
    expect(isArchiveDrop(["C:\\Downloads\\chart.rar"])).toBe(true);
    expect(isArchiveDrop(["C:\\Downloads\\chart.zip.exe"])).toBe(false);
    expect(isArchiveDrop(["C:\\Downloads\\chart.zip "] )).toBe(false);
    expect(isArchiveDrop(["first.zip", "second.zip"])).toBe(true);
    expect(isArchiveDrop(["chart.zip", "notes.txt"])).toBe(false);
    expect(isArchiveDrop([])).toBe(false);
  });
});

describe("installed chart recognition", () => {
  it("recognizes a manual chart by title, artist, charter, and duration", () => {
    expect(chartMatchesInstalledChart(chart, installed)).toBe(true);
  });

  it("does not confuse the same song charted by someone else", () => {
    expect(chartMatchesInstalledChart(chart, { ...installed, charter: "another charter" })).toBe(false);
  });

  it("uses a managed chart id as the authority", () => {
    expect(chartMatchesInstalledChart(chart, { ...installed, chartId: chart.id, managed: true })).toBe(true);
    expect(chartMatchesInstalledChart(chart, {
      ...installed,
      chartId: "503c0c85-d4b6-4366-b18b-bbcc6fb44f63",
      managed: true
    })).toBe(false);
  });
});
