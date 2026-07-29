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

describe("chart ZIP drag and drop", () => {
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
    mocks.invoke.mockImplementation(async (command: string) => {
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
        return {
          archivePath: "C:\\Downloads\\chart.zip",
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
    expect(screen.getByText("drop to import chart")).toBeInTheDocument();

    act(() => mocks.dragHandler?.({ payload: { type: "leave" } }));
    expect(screen.queryByText("drop to import chart")).not.toBeInTheDocument();
  });

  it("drops a ZIP into native validation and opens downloads", async () => {
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

  it("rejects multiple files before native validation", async () => {
    render(<App />);
    await waitFor(() => expect(mocks.dragHandler).not.toBeNull());

    act(() => mocks.dragHandler?.({
      payload: { type: "drop", paths: ["first.zip", "second.zip"] }
    }));

    expect(await screen.findByText("Drop one ZIP chart archive at a time.")).toBeInTheDocument();
    expect(mocks.invoke).not.toHaveBeenCalledWith(
      "inspect_chart_archive",
      expect.anything()
    );
  });
});
