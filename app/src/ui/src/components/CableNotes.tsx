// VB-Audio's virtual cables, suggested where they are missing: the Hi-Fi
// Cable for the sound (it adds the least delay), VB-CABLE for the headphones'
// microphone. Install VB-CABLE has the core fetch VB-Audio's own pack from
// VB-Audio and run its setup (BlueCaster ships none of it); VB-Audio asks
// that the user can tell it is their donationware, so its origin is said
// beside the button (https://vb-audio.com/Services/licensing.htm). Where the
// download fails, VB-Audio's page is offered instead. The Hi-Fi Cable is
// only linked. Each suggestion can be closed; Settings keeps both.

import { createSignal, type JSX, Show } from "solid-js";
import { useApp } from "../app/context";
import { saveSettings } from "../app/settings";
import { t } from "../i18n";
import type { AudioDevice } from "../ipc/types";
import { LINKS } from "../lib/links";
import { hifiCable, vbCable } from "../lib/profile";
import { Button, IconButton } from "./controls";
import { Icon } from "./Icon";

/** VB-Audio's name for it, and that it is donationware: a link to its site */
function Origin(props: { text: string; url: string }) {
  const { core } = useApp();
  return (
    <p class="hint cable-origin">
      {props.text}{" "}
      <a
        href={props.url}
        onClick={(e) => {
          e.preventDefault();
          core.transport.openUrl(props.url);
        }}
      >
        {props.url.replace(/^https:\/\//, "").replace(/\/$/, "")}
      </a>
    </p>
  );
}

/** VB-CABLE's Install button, and VB-Audio's page once it failed.
 *  `onInstalled` once it is among Windows' outputs. */
export function VbCableInstall(props: { onInstalled?: () => void }) {
  const app = useApp();
  const { core } = app;
  const [busy, setBusy] = createSignal(false);
  // It installed but Windows does not list it yet
  const [restart, setRestart] = createSignal(false);
  // It failed (no download, VB-Audio changed the pack, ...): their page
  const [failed, setFailed] = createSignal(false);
  const install = async () => {
    setBusy(true);
    try {
      let found = false;
      const ok = await app.run(async () => {
        found = await core.request("audio.install_vb_cable");
      });
      if (ok && found) props.onInstalled?.();
      else if (ok) setRestart(true);
      else setFailed(true);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button icon="arrow_download" busy={busy()} onClick={() => void install()}>
        {t("vbcable.install")}
      </Button>
      <Show when={failed()}>
        <Button icon="open" onClick={() => core.transport.openUrl(LINKS.vbCable)}>
          {t("vbcable.download")}
        </Button>
      </Show>
      <Show when={restart()}>
        <p class="field-warn">
          <Icon name="warning" />
          <span>{t("vbcable.restart")}</span>
        </p>
      </Show>
    </>
  );
}

export function VbCableNote(props: { onInstalled?: () => void; onClose?: () => void }) {
  return (
    <div class="setup-note cable-note">
      <div class="cable-note-text">
        <p>{t("ui.setup.vbcable_note")}</p>
        <Origin text={t("vbcable.origin")} url={LINKS.vbCableHome} />
      </div>
      <div class="cable-note-actions">
        <VbCableInstall onInstalled={props.onInstalled} />
        <Show when={props.onClose}>
          <IconButton icon="dismiss" label={t("modal.close")} onClick={() => props.onClose?.()} />
        </Show>
      </div>
    </div>
  );
}

export function HifiCableNote(props: { onClose?: () => void }) {
  const { core } = useApp();
  return (
    <div class="setup-note cable-note">
      <div class="cable-note-text">
        <p>{t("ui.setup.hificable_note")}</p>
        <Origin text={t("hificable.origin")} url={LINKS.vbAudioHome} />
      </div>
      <div class="cable-note-actions">
        <Button icon="open" onClick={() => core.transport.openUrl(LINKS.hifiCable)}>
          {t("hificable.download")}
        </Button>
        <Show when={props.onClose}>
          <IconButton icon="dismiss" label={t("modal.close")} onClick={() => props.onClose?.()} />
        </Show>
      </div>
    </div>
  );
}

/** The suggestions for the cables Windows does not have (`devices`: its
 *  outputs, null until read), each until closed. `wrap` puts each in its
 *  own card (Audio Devices). */
export function CableNotes(props: { devices: readonly AudioDevice[] | null; reload: () => void; wrap?: boolean }) {
  const app = useApp();
  const s = () => app.core.state.snap!.settings;
  const noVb = () => props.devices !== null && vbCable(props.devices) === null && !s().vb_cable_note_hidden;
  const noHifi = () => props.devices !== null && hifiCable(props.devices) === null && !s().hifi_cable_note_hidden;
  const card = (note: () => JSX.Element) =>
    props.wrap ? <section class="card device-note">{note()}</section> : note();
  return (
    <>
      <Show when={noHifi()}>
        {card(() => (
          <HifiCableNote onClose={() => void saveSettings(app, { hifi_cable_note_hidden: true })} />
        ))}
      </Show>
      <Show when={noVb()}>
        {card(() => (
          <VbCableNote
            onInstalled={props.reload}
            onClose={() => void saveSettings(app, { vb_cable_note_hidden: true })}
          />
        ))}
      </Show>
    </>
  );
}

/** Settings' rows: either cable, installed or not, also after its suggestion
 *  was closed */
export function CableSettings() {
  const app = useApp();
  const [devices, setDevices] = createSignal<AudioDevice[] | null>(null);
  const load = () =>
    app.core
      .request("audio.devices")
      .then(setDevices)
      .catch(() => {});
  void load();
  const row = (name: string, installed: () => boolean, action: () => JSX.Element, origin: () => JSX.Element) => (
    <div class="row">
      <span class="row-text">
        <span class="row-label">{name}</span>
        {origin()}
      </span>
      <Show when={devices() !== null}>
        <Show when={!installed()} fallback={<span class="hint">{t("vbcable.installed")}</span>}>
          {action()}
        </Show>
      </Show>
    </div>
  );
  return (
    <>
      {row(
        "Hi-Fi Cable",
        () => hifiCable(devices() ?? []) !== null,
        () => (
          <Button icon="open" onClick={() => app.core.transport.openUrl(LINKS.hifiCable)}>
            {t("hificable.download")}
          </Button>
        ),
        () => <Origin text={t("hificable.origin")} url={LINKS.vbAudioHome} />,
      )}
      {row(
        "VB-CABLE",
        () => vbCable(devices() ?? []) !== null,
        () => <VbCableInstall onInstalled={() => void load()} />,
        () => <Origin text={t("vbcable.origin")} url={LINKS.vbCableHome} />,
      )}
    </>
  );
}
