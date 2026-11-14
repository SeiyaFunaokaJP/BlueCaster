import { unwrap } from "solid-js/store";
import type { Settings, SettingsApplied } from "../ipc/types";
import type { AppContext } from "./context";

/** The Volume Mixer (screen, flyout list, output mute): Developer > Volume
 *  Mixer on, in developer mode (issue #86) */
export function mixerShown(s: Pick<Settings, "developer_mode" | "volume_mixer"> | undefined): boolean {
  return !!s?.developer_mode && s.volume_mixer;
}

/** The Output and Input Source lists offer every device (each output, the
 *  16-channel cable, the other side's cable): Developer > Show every audio
 *  device on, in developer mode (issue #86: the lists read as chaos) */
export function allAudioDevices(s: Pick<Settings, "developer_mode" | "all_audio_devices"> | undefined): boolean {
  return !!s?.developer_mode && s.all_audio_devices;
}

/** A device's "Set as Windows default output/input while connected" in
 *  force: shown and as saved in developer mode, on for everyone else (issue
 *  #86: apps without an output picker and the microphone need them; the
 *  core does the same) */
export function devicesSwitchShown(s: Pick<Settings, "developer_mode"> | undefined): boolean {
  return !!s?.developer_mode;
}
export function defaultDeviceSwitch(s: Pick<Settings, "developer_mode"> | undefined, saved: boolean | undefined): boolean {
  return saved !== false || !devicesSwitchShown(s);
}

/** Changes some settings: the core takes the whole object and sends it back as an event */
export function saveSettings(app: AppContext, patch: Partial<Settings>): Promise<boolean> {
  return applySettings(app, patch).then((r) => r !== null);
}

/** As saveSettings, with what the core did besides saving (null: it failed, already told) */
export async function applySettings(app: AppContext, patch: Partial<Settings>): Promise<SettingsApplied | null> {
  const current = unwrap(app.core.state.snap!.settings);
  let applied: SettingsApplied | null = null;
  const ok = await app.run(async () => {
    // An older core answers null
    applied = (await app.core.request("settings.set", { settings: { ...current, ...patch } })) ?? {
      le_audio_deferred: false,
    };
  });
  return ok ? applied : null;
}
