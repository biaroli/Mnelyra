import { writable } from "svelte/store";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { APP_VERSION } from "$lib/app-version";

export type UpdatePhase =
  | "idle"
  | "checking"
  | "available"
  | "up-to-date"
  | "downloading"
  | "installing"
  | "error";

export interface UpdateState {
  phase: UpdatePhase;
  currentVersion: string;
  latestVersion: string | null;
  notes: string | null;
  date: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  error: string | null;
}

const initialState: UpdateState = {
  phase: "idle",
  currentVersion: APP_VERSION,
  latestVersion: null,
  notes: null,
  date: null,
  downloadedBytes: 0,
  totalBytes: null,
  error: null,
};

export const updateState = writable<UpdateState>(initialState);

let availableUpdate: Update | null = null;
let checkPromise: Promise<UpdateState> | null = null;

function setState(next: UpdateState): UpdateState {
  updateState.set(next);
  return next;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export async function checkForUpdates(): Promise<UpdateState> {
  if (checkPromise) return checkPromise;

  checkPromise = (async () => {
    updateState.update((state) => ({ ...state, phase: "checking", error: null }));
    try {
      if (availableUpdate) {
        await availableUpdate.close().catch(() => undefined);
        availableUpdate = null;
      }

      const update = await check({ timeout: 15_000 });
      if (!update) {
        return setState({
          ...initialState,
          phase: "up-to-date",
          currentVersion: APP_VERSION,
        });
      }

      availableUpdate = update;
      return setState({
        ...initialState,
        phase: "available",
        currentVersion: update.currentVersion || APP_VERSION,
        latestVersion: update.version,
        notes: update.body?.trim() || null,
        date: update.date || null,
      });
    } catch (error) {
      return setState({
        ...initialState,
        phase: "error",
        error: errorText(error),
      });
    } finally {
      checkPromise = null;
    }
  })();

  return checkPromise;
}

export async function installAvailableUpdate(): Promise<void> {
  const update = availableUpdate;
  if (!update) {
    throw new Error("没有可安装的更新，请先重新检查。");
  }

  let downloadedBytes = 0;
  let totalBytes: number | null = null;
  updateState.update((state) => ({
    ...state,
    phase: "downloading",
    downloadedBytes: 0,
    totalBytes: null,
    error: null,
  }));

  const onEvent = (event: DownloadEvent) => {
    if (event.event === "Started") {
      totalBytes = event.data.contentLength ?? null;
      downloadedBytes = 0;
    } else if (event.event === "Progress") {
      downloadedBytes += event.data.chunkLength;
    } else if (event.event === "Finished") {
      updateState.update((state) => ({ ...state, phase: "installing" }));
      return;
    }

    updateState.update((state) => ({
      ...state,
      downloadedBytes,
      totalBytes,
    }));
  };

  try {
    await update.downloadAndInstall(onEvent);
    updateState.update((state) => ({ ...state, phase: "installing" }));
    await relaunch();
  } catch (error) {
    updateState.update((state) => ({
      ...state,
      phase: "error",
      error: errorText(error),
    }));
    throw error;
  }
}
