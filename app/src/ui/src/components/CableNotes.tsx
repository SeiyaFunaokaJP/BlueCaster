// VB-Audio's virtual cables: the Hi-Fi Cable for the sound (it adds the
// least delay), VB-CABLE for the headphones' microphone. The pages show one
// amber line while a cable is missing; Learn more opens the dialog with both
// cables, each with Install (Download for the Hi-Fi Cable) or Uninstall, and
// VB-Audio's links. Nothing else in the app talks about the cables (issue
// #86). Install VB-CABLE has the core fetch VB-Audio's own pack from
// VB-Audio and run its setup (BlueCaster ships none of it); Uninstall runs
// VB-Audio's installed setup. VB-Audio asks that the user can tell it is
// their donationware, so its origin is said under each cable
// (https://vb-audio.com/Services/licensing.htm). The line cannot be closed
// (issue #86: other ways to the microphone only bring bug reports);
// Developer > Hide the virtual sound devices warning removes it. About >
// Third-party software opens the same dialog, the only place that names
// the cables' origin.

import { createSignal, Show } from "solid-js";
import { useApp } from "../app/context";
import { t } from "../i18n";
import type { AudioDevice, Settings } from "../ipc/types";
import { LINKS } from "../lib/links";
import { hifiCable, vbCable } from "../lib/profile";
import { Button, Dialog } from "./controls";
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

/** One cable: its name with Installed or Not installed, its origin, and
 *  Install or Uninstall.
 *  `installed` null until Windows' outputs are read. */
function CableRow(props: {
  name: string;
  origin: string;
  originUrl: string;
  installed: boolean | null;
  hifi: boolean;
  onChanged: () => void;
}) {
  const app = useApp();
  const { core } = app;
  const [busy, setBusy] = createSignal(false);
  // Done, but Windows still lists it, or does not list it yet
  const [restart, setRestart] = createSignal(false);
  // Install failed (no download, VB-Audio changed the pack, ...): their page
  const [failed, setFailed] = createSignal(false);
  const act = async (uninstall: boolean) => {
    setBusy(true);
    setRestart(false);
    try {
      let done = false;
      const ok = await app.run(async () => {
        done = uninstall
          ? await core.request("audio.uninstall_cable", { hifi: props.hifi })
          : await core.request("audio.install_vb_cable");
      });
      if (ok && done) props.onChanged();
      else if (ok) setRestart(true);
      else if (!uninstall) setFailed(true);
    } finally {
      setBusy(false);
    }
  };
  const page = () => core.transport.openUrl(props.hifi ? LINKS.hifiCable : LINKS.vbCable);
  return (
    <div class="cable-row">
      <div class="cable-row-text">
        <div class="cable-row-name">
          <strong>{props.name}</strong>
          <Show when={props.installed !== null}>
            <span class={`tag ${props.installed ? "st-good" : "st-warn"}`}>
              {t(props.installed ? "cables.installed" : "cables.not_installed")}
            </span>
          </Show>
        </div>
        <Origin text={props.origin} url={props.originUrl} />
      </div>
      <div class="cable-row-actions">
        <Show when={props.installed !== null}>
          <Show
            when={props.installed}
            fallback={
              <Show
                when={!props.hifi}
                fallback={
                  <Button icon="open" onClick={page}>
                    {t("cables.download")}
                  </Button>
                }
              >
                <Button icon="arrow_download" busy={busy()} onClick={() => void act(false)}>
                  {t("cables.install")}
                </Button>
                <Show when={failed()}>
                  <Button icon="open" onClick={page}>
                    {t("cables.download")}
                  </Button>
                </Show>
              </Show>
            }
          >
            <Button busy={busy()} onClick={() => void act(true)}>
              {t("cables.uninstall")}
            </Button>
          </Show>
        </Show>
      </div>
      <Show when={restart()}>
        <p class="field-warn cable-row-note">
          <Icon name="warning" />
          <span>{t("cables.restart")}</span>
        </p>
      </Show>
    </div>
  );
}

/** Both cables and VB-Audio's links (`devices`: Windows' outputs, null
 *  until read; `reload` reads them again) */
export function CablesDialog(props: {
  open: boolean;
  devices: readonly AudioDevice[] | null;
  reload: () => void;
  onClose: () => void;
}) {
  const { core } = useApp();
  const has = (find: (d: readonly AudioDevice[]) => AudioDevice | null) =>
    props.devices === null ? null : find(props.devices) !== null;
  const link = (url: string, label: string) => (
    <a
      href={url}
      onClick={(e) => {
        e.preventDefault();
        core.transport.openUrl(url);
      }}
    >
      {label}
    </a>
  );
  return (
    <Dialog open={props.open} title={t("cables.title")} onCancel={props.onClose}>
      <div class="cable-dialog">
        <div>
          <p>{t("cables.intro")}</p>
          <ul class="cable-reasons">
            <li>{t("cables.reason_separate")}</li>
            <li>{t("cables.reason_features")}</li>
          </ul>
        </div>
        <CableRow
          name="VB-CABLE"
          origin={t("vbcable.origin")}
          originUrl={LINKS.vbCableHome}
          installed={has(vbCable)}
          hifi={false}
          onChanged={props.reload}
        />
        <CableRow
          name="Hi-Fi Cable"
          origin={t("hificable.origin")}
          originUrl={LINKS.vbAudioHome}
          installed={has(hifiCable)}
          hifi
          onChanged={props.reload}
        />
        <hr class="cable-rule" />
        <div class="cable-vendor">
          <strong>VB-Audio</strong>
          <p class="hint inline wrap">
            {link(LINKS.vbAudioHome, "vb-audio.com")}
            {link(LINKS.vbCableHome, "vb-cable.com")}
            {link(LINKS.vbAudioLicensing, t("cables.licensing"))}
          </p>
        </div>
      </div>
    </Dialog>
  );
}

/** The warning hidden on the Developer page (developer mode only) */
export function cableWarningHidden(
  s: Pick<Settings, "developer_mode" | "vb_cable_note_hidden" | "hifi_cable_note_hidden">,
): boolean {
  return s.developer_mode && (s.vb_cable_note_hidden || s.hifi_cable_note_hidden);
}

/** One amber line while a cable is missing (`devices`: Windows' outputs,
 *  null until read); Learn more opens both cables. It cannot be closed. */
export function CableNotes(props: { devices: readonly AudioDevice[] | null; reload: () => void }) {
  const app = useApp();
  const s = () => app.core.state.snap!.settings;
  const [open, setOpen] = createSignal(false);
  const missing = () =>
    props.devices !== null && (vbCable(props.devices) === null || hifiCable(props.devices) === null);
  // Kept while the dialog is open: installing the last cable would close it
  const shown = () => (missing() || open()) && !cableWarningHidden(s());
  return (
    <Show when={shown()}>
      <Show when={missing()}>
        <div class="banner banner-warn cable-banner">
          <Icon name="warning" class="banner-icon" />
          <span class="cable-banner-text">{t("cables.recommended")}</span>
          <Button onClick={() => setOpen(true)}>{t("cables.learn_more")}</Button>
        </div>
      </Show>
      <CablesDialog open={open()} devices={props.devices} reload={props.reload} onClose={() => setOpen(false)} />
    </Show>
  );
}
