import { describe, expect, it } from "vitest";
import { buildChartCoverUrl, buildChartPreviewUrl, buildChartPublicUrl, chartContentVersion, difficultyClass, formatBytes, formatDuration } from "./lib";

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
});
