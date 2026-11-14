// VB-Audio's virtual cables: the Hi-Fi Cable for the sound (it adds the
// least delay), VB-CABLE for the headphones' microphone. The pages show one
// amber line while a cable is missing, a green one once both are installed
// (issue #86: kept, with its Learn more); Learn more opens the dialog with both
// cables, each with Install (Download for the Hi-Fi Cable) or Uninstall, and
// VB-Audio's links. Nothing else in the app talks about the cables (issue
// #86). Install VB-CABLE has the core fetch VB-Audio's own pack from
// VB-Audio and run its setup (BlueCaster ships none of it); Uninstall runs
// VB-Audio's installed setup. VB-Audio asks that the user can tell it is
// their donationware, so its origin is said under each cable
// (https://vb-audio.com/Services/licensing.htm). The line cannot be closed
// (issue #86: other ways to the microphone only bring bug reports);
// Developer > Hide the virtual sound devices warning removes it, green too. About >
// Third-party software opens the same dialog, the only place that names
// the cables' origin. After an update from a version without it, the dialog
// opens by itself once on the main window (issue #86: the setup, which shows
// the line, does not run again).

import { createEffect, createSignal, on, onCleanup, onMount, Show } from "solid-js";
import { errorText, onAudioDevicesChanged, useApp } from "../app/context";
import { saveSettings } from "../app/settings";
import { t } from "../i18n";
import { IpcError, type AudioDevice, type Settings } from "../ipc/types";
import { LINKS } from "../lib/links";
import { hifiCable, vbCable } from "../lib/profile";
import { Button, Dialog } from "./controls";
import { Icon } from "./Icon";

/** How long Windows' outputs are read again after an install or removal
 *  that did not see the cable change: every 2 s for 30 s */
const POLL_MS = 2000;
const POLLS = 15;

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
  // Windows' outputs read again for a while: a cable may show up (or go)
  // after the answer (issue #86: installed, yet the error and Download stayed)
  let polls = 0;
  let timer: ReturnType<typeof setInterval> | undefined;
  const stopPolling = () => {
    clearInterval(timer);
    timer = undefined;
  };
  const poll = () => {
    stopPolling();
    polls = 0;
    timer = setInterval(() => {
      if (++polls > POLLS) stopPolling();
      else props.onChanged();
    }, POLL_MS);
  };
  onCleanup(stopPolling);
  // The cable came or went: what the last try said no longer holds
  createEffect(
    on(
      () => props.installed,
      () => {
        stopPolling();
        setRestart(false);
        setFailed(false);
      },
      { defer: true },
    ),
  );
  const act = async (uninstall: boolean) => {
    setBusy(true);
    setRestart(false);
    setFailed(false);
    stopPolling();
    try {
      const done = uninstall
        ? await core.request("audio.uninstall_cable", { hifi: props.hifi })
        : await core.request("audio.install_vb_cable");
      if (!done) {
        setRestart(true);
        poll();
      }
    } catch (e) {
      // No answer in time: the setup may still be running
      if (IpcError.from(e).code === "ipc.timeout") {
        setRestart(true);
        poll();
      } else {
        app.notify(errorText(e), "error");
        if (!uninstall) setFailed(true);
      }
    } finally {
      setBusy(false);
      // Always: an answer that came too late or said "not yet" may be wrong
      props.onChanged();
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
        <hr class="cable-rule" />
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

/** One amber line while a cable is missing, a green one once both are
 *  installed (`devices`: Windows' outputs, null until read); Learn more
 *  opens both cables. It cannot be closed. */
export function CableNotes(props: { devices: readonly AudioDevice[] | null; reload: () => void }) {
  const app = useApp();
  const s = () => app.core.state.snap!.settings;
  const [open, setOpen] = createSignal(false);
  const missing = () =>
    props.devices !== null && (vbCable(props.devices) === null || hifiCable(props.devices) === null);
  const shown = () => props.devices !== null && !cableWarningHidden(s());
  return (
    <Show when={shown()}>
      <div class={`banner ${missing() ? "banner-warn" : "banner-good"} cable-banner`}>
        <Icon name={missing() ? "warning" : "checkmark_circle"} class="banner-icon" />
        <span class="cable-banner-text">{t(missing() ? "cables.recommended" : "cables.installed_both")}</span>
        <Button onClick={() => setOpen(true)}>{t("cables.learn_more")}</Button>
      </div>
      <CablesDialog open={open()} devices={props.devices} reload={props.reload} onClose={() => setOpen(false)} />
    </Show>
  );
}

/** The dialog once by itself after an update (settings.cables_dialog_shown:
 *  false in a folder set up before it). Both cables installed, or the
 *  warning hidden in developer mode: nothing to show, it only counts as
 *  shown. Closed: never again. */
export function CablesOnce() {
  const app = useApp();
  const { core } = app;
  const [devices, setDevices] = createSignal<AudioDevice[] | null>(null);
  const [open, setOpen] = createSignal(false);
  const load = () =>
    core
      .request("audio.devices")
      .then(setDevices)
      .catch(() => {});
  const shown = () => void saveSettings(app, { cables_dialog_shown: true });
  onMount(() => {
    const s = core.state.snap!.settings;
    // A first setup shows the line itself
    if (s.cables_dialog_shown || !s.setup_done) return;
    if (cableWarningHidden(s)) return shown();
    core
      .request("audio.devices")
      .then((list) => {
        setDevices(list);
        if (vbCable(list) !== null && hifiCable(list) !== null) shown();
        else setOpen(true);
      })
      // Not read: next time
      .catch(() => {});
  });
  onAudioDevicesChanged(() => {
    if (open()) void load();
  });
  return (
    <CablesDialog
      open={open()}
      devices={devices()}
      reload={() => void load()}
      onClose={() => {
        setOpen(false);
        shown();
      }}
    />
  );
}
