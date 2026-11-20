// Audio Devices: what holds for every audio device (Windows Volume Sync, the
// buttons, the remembered volume: one switch each, issue #86), then the
// headphones and speakers BlueCaster knows, each with its own settings (capture
// source, default output switching), and a card to add one: scan for new
// ones or type an address. Profiles then pick from this list.

import { createSignal, onCleanup, onMount } from "solid-js";
import { onAudioDevicesChanged, useApp } from "../app/context";
import { saveSettings } from "../app/settings";
import { Toggle } from "../components/controls";
import { DevicePicker } from "../components/DevicePicker";
import { CableNotes } from "../components/CableNotes";
import { DeviceSettings } from "../components/DeviceSettings";
import { t } from "../i18n";
import type { AudioDevice } from "../ipc/types";

export function Devices() {
  const { core } = useApp();
  // Windows' outputs, for the cables it lacks (installed meanwhile: read
  // again when the window comes back)
  const [devices, setDevices] = createSignal<AudioDevice[] | null>(null);
  const load = () =>
    core
      .request("audio.devices")
      .then(setDevices)
      .catch(() => {});
  onMount(() => {
    void load();
    window.addEventListener("focus", load);
  });
  onCleanup(() => window.removeEventListener("focus", load));
  onAudioDevicesChanged(() => void load());
  return (
    <div class="page">
      <header class="page-head">
        <h1>{t("ui.nav.devices")}</h1>
      </header>
      <CableNotes devices={devices()} reload={() => void load()} />
      <AllDevices />
      <DevicePicker manage address="" onPick={() => {}} settings={(d) => <DeviceSettings device={d} />} />
    </div>
  );
}

/** The settings shared by every audio device (Settings, saved at once; a
 *  Volume Sync or output mute change starts the profile in use again in the core) */
function AllDevices() {
  const app = useApp();
  const s = () => app.core.state.snap!.settings;
  return (
    <section class="card device-global" aria-labelledby="devices-all">
      <h2 id="devices-all" class="mini-title">
        {t("ui.devices.all_title")}
      </h2>
      <Toggle
        label={t("capture.volume_sync")}
        hint={t("capture.volume_sync_hint")}
        checked={s().volume_sync}
        onChange={(v) => void saveSettings(app, { volume_sync: v })}
      />
      <Toggle
        label={t("settings.media_keys")}
        checked={s().media_keys}
        onChange={(v) => void saveSettings(app, { media_keys: v })}
      />
      <Toggle
        label={t("settings.auto_reconnect")}
        checked={s().auto_reconnect}
        onChange={(v) => void saveSettings(app, { auto_reconnect: v })}
      />
    </section>
  );
}
