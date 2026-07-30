// @vitest-environment jsdom

import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  dragHandler: null as null | ((event: {
    payload:
      | { type: "enter"; paths: string[] }
      | { type: "over" }
      | { type: "drop"; paths: string[] }
      | { type: "leave" };
  }) => void),
  invoke: vi.fn()
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => vi.fn())
}));

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (handler: typeof mocks.dragHandler) => {
      mocks.dragHandler = handler;
      return vi.fn();
    }
  })
}));

vi.mock("@tauri-apps/plugin-deep-link", () => ({
  getCurrent: vi.fn(async () => []),
  onOpenUrl: vi.fn(async () => vi.fn())
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
  save: vi.fn()
}));

import App from "./App";

const appState = {
  customSongsPath: "C:\\CustomSongs",
  directoryExists: true
};

describe("chart archive drag and drop", () => {
  beforeEach(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {}
    });
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        addEventListener: vi.fn(),
        matches: false,
        removeEventListener: vi.fn()
      }))
    });
    localStorage.clear();
    localStorage.setItem("unchartable:custom-songs", appState.customSongsPath);
    mocks.dragHandler = null;
    mocks.invoke.mockImplementation(async (command: string, args?: { archivePath?: string }) => {
      if (command === "get_app_state" || command === "validate_custom_songs_path") return appState;
      if (command === "fetch_charts") return { charts: [], count: 0, nextPage: null };
      if (command === "fetch_packs") return { packs: [], count: 0, nextPage: null };
      if ([
        "list_installed_charts",
        "list_trashed_charts",
        "list_chart_backups",
        "find_manual_chart_matches",
        "check_installed_updates"
      ].includes(command)) return [];
      if (command === "inspect_chart_archive") {
        const archivePath = args?.archivePath || "C:\\Downloads\\chart.zip";
        return {
          archivePath,
          archiveFormat: archivePath.split(".").pop() || "zip",
          archiveSizeBytes: 1024,
          artist: "Test Artist",
          charter: "Test Charter",
          conflictPath: null,
          title: "Dropped Chart"
        };
      }
      return undefined;
    });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("shows and clears the valid drop overlay", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.dragHandler).not.toBeNull());

    act(() => mocks.dragHandler?.({
      payload: { type: "enter", paths: ["C:\\Downloads\\chart.ZIP"] }
    }));
    expect(screen.getByText("drop to import charts")).toBeInTheDocument();

    act(() => mocks.dragHandler?.({ payload: { type: "leave" } }));
    expect(screen.queryByText("drop to import charts")).not.toBeInTheDocument();
  });

  it("drops a supported archive into native validation and opens downloads", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.dragHandler).not.toBeNull());

    act(() => mocks.dragHandler?.({
      payload: { type: "drop", paths: ["C:\\Downloads\\chart.zip"] }
    }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("inspect_chart_archive", {
      archivePath: "C:\\Downloads\\chart.zip",
      targetDirectory: "C:\\CustomSongs"
    }));
    expect(await screen.findByText("Dropped Chart")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "your charts" })).toBeInTheDocument();
  });

  it("accepts RAR archives for native validation", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.dragHandler).not.toBeNull());

    act(() => mocks.dragHandler?.({
      payload: { type: "drop", paths: ["C:\\Downloads\\chart.rar"] }
    }));

    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("inspect_chart_archive", {
      archivePath: "C:\\Downloads\\chart.rar",
      targetDirectory: "C:\\CustomSongs"
    }));
  });

  it("validates multiple archives before batch import", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.dragHandler).not.toBeNull());

    act(() => mocks.dragHandler?.({
      payload: { type: "drop", paths: ["first.zip", "second.zip"] }
    }));

    await waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("inspect_chart_archive", {
        archivePath: "first.zip",
        targetDirectory: "C:\\CustomSongs"
      });
      expect(mocks.invoke).toHaveBeenCalledWith("inspect_chart_archive", {
        archivePath: "second.zip",
        targetDirectory: "C:\\CustomSongs"
      });
    });
    expect(await screen.findByText("2 archives checked")).toBeInTheDocument();
  });
});
