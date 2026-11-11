// VB-Audio's virtual cables, suggested where they are missing: the Hi-Fi
// Cable for the sound (it adds the least delay), VB-CABLE for the headphones'
// microphone. Install VB-CABLE has the core fetch VB-Audio's own pack from
// VB-Audio and run its setup (BlueCaster ships none of it); VB-Audio asks
// that the user can tell it is their donationware, so its origin is said
// beside the button (https://vb-audio.com/Services/licensing.htm). Where the
// download fails, VB-Audio's page is offered instead. The Hi-Fi Cable is
// only linked. The pages show one line and keep the rest in a dialog. The
// line stays while a cable is missing (issue #86: other ways to the
// microphone only bring bug reports); Developer > Hide the virtual sound
// devices warning removes it. About names both as VB-Audio's donationware.

import { createSignal, Show } from "solid-js";
import { useApp } from "../app/context";
import { t } from "../i18n";
import type { AudioDevice, Settings } from "../ipc/types";
import { LINKS } from "../lib/links";
import { hifiCable, vbCable } from "../lib/profile";
import { Button, Dialog } from "./controls";
import { Icon } from "./Icon";

/** VB-Audio's name for it, and that it is donationware: a link to its site */
export function Origin(props: { text: string; url: string }) {
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
function VbCableInstall(props: { onInstalled?: () => void }) {
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

function VbCableNote(props: { onInstalled?: () => void }) {
  return (
    <div class="setup-note cable-note">
      <div class="cable-note-text">
        <p>{t("ui.setup.vbcable_note")}</p>
        <Origin text={t("vbcable.origin")} url={LINKS.vbCableHome} />
      </div>
      <div class="cable-note-actions">
        <VbCableInstall onInstalled={props.onInstalled} />
      </div>
    </div>
  );
}

function HifiCableNote() {
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
      </div>
    </div>
  );
}

/** The warning hidden on the Developer page (developer mode only) */
export function cableWarningHidden(
  s: Pick<Settings, "developer_mode" | "vb_cable_note_hidden" | "hifi_cable_note_hidden">,
): boolean {
  return s.developer_mode && (s.vb_cable_note_hidden || s.hifi_cable_note_hidden);
}

/** One amber line while a cable is missing (`devices`: Windows' outputs,
 *  null until read); Learn more opens the suggestions for the missing ones
 *  (issue #86: the notes made the pages too busy). It cannot be closed. */
export function CableNotes(props: { devices: readonly AudioDevice[] | null; reload: () => void }) {
  const app = useApp();
  const s = () => app.core.state.snap!.settings;
  const [open, setOpen] = createSignal(false);
  const noVb = () => props.devices !== null && vbCable(props.devices) === null;
  const noHifi = () => props.devices !== null && hifiCable(props.devices) === null;
  const shown = () => (noVb() || noHifi()) && !cableWarningHidden(s());
  return (
    <Show when={shown()}>
      <div class="banner banner-warn cable-banner">
        <Icon name="warning" class="banner-icon" />
        <span class="cable-banner-text">{t("cables.recommended")}</span>
        <Button onClick={() => setOpen(true)}>{t("cables.learn_more")}</Button>
      </div>
      <Dialog open={open()} title={t("cables.title")} onCancel={() => setOpen(false)}>
        <div class="cable-dialog">
          <Show when={noHifi()}>
            <HifiCableNote />
          </Show>
          <Show when={noVb()}>
            <VbCableNote onInstalled={props.reload} />
          </Show>
        </div>
      </Dialog>
    </Show>
  );
}

/** About: both cables named as VB-Audio's donationware, with their sites */
export function CableOrigins() {
  return (
    <>
      <Origin text={t("hificable.origin")} url={LINKS.vbAudioHome} />
      <Origin text={t("vbcable.origin")} url={LINKS.vbCableHome} />
    </>
  );
}
