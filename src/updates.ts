// Updates (G06): a check at startup when "Update automatically" is on, or
// from Settings › About. A found update is downloaded in the background and
// installed only when the user restarts: on Windows the installer closes the
// app, so it never runs unasked.
import { create } from "zustand";
import { api, errorMessage, inTauri, type UpdateInfo } from "./lib/api";
import { usePrefs } from "./prefs";

export type UpdateState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "current"; checkedAt: number }
  | { status: "downloading"; info: UpdateInfo; downloaded: number; total: number | null }
  | { status: "ready"; info: UpdateInfo }
  | { status: "installing"; info: UpdateInfo }
  | { status: "failed"; message: string };

export const useUpdates = create<UpdateState & { dismissed: boolean }>(() => ({ status: "idle", dismissed: false }));

const set = (state: UpdateState, dismissed = false) => useUpdates.setState({ ...state, dismissed }, true);

/** Looks for a newer version and, when there is one, downloads it. */
export async function checkForUpdates() {
  const now = useUpdates.getState();
  if (now.status === "checking" || now.status === "downloading" || now.status === "installing") return;
  set({ status: "checking" });
  try {
    const info = await api.checkUpdate();
    if (!info) return set({ status: "current", checkedAt: Date.now() });
    set({ status: "downloading", info, downloaded: 0, total: null });
    await api.downloadUpdate((downloaded, total) => set({ status: "downloading", info, downloaded, total }));
    set({ status: "ready", info });
  } catch (e) {
    set({ status: "failed", message: errorMessage(e) });
  }
}

/** Installs the downloaded update; the app starts again in the new version. */
export async function installUpdate() {
  const now = useUpdates.getState();
  if (now.status !== "ready") return;
  set({ status: "installing", info: now.info });
  try {
    await api.installUpdate();
  } catch (e) {
    set({ status: "failed", message: errorMessage(e) });
  }
}

/** Hides the notice until the next start; the update stays downloaded. */
export const dismissUpdate = () => useUpdates.setState({ dismissed: true });

/** At startup, a few seconds in, so opening the app is not slowed down. */
export function checkAtStartup() {
  if (!inTauri || !usePrefs.getState().updateAutomatically) return;
  window.setTimeout(() => {
    // A failed check at startup stays quiet: no network is not worth a notice.
    void checkForUpdates().then(() => {
      if (useUpdates.getState().status === "failed") set({ status: "idle" });
    });
  }, 4000);
}
