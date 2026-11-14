// A device's Input Source on Audio Devices (issue #86; it was on Codecs):
// the headphones' microphone rendered to an output of the PC, a virtual
// cable whose recording side apps use as a microphone. Off, Auto (only
// while an app records from it) or On, with the mode beside the device (below it
// when narrow; issue #86: not above it), the device always there (disabled
// while Off) so that
// switching does not move the page. The list names each device by the side
// apps pick ("CABLE Output", not "CABLE Input"; an output whose recording
// side another one already names, with its own name added). Only the cables
// a microphone can use: not the 16-channel one, outputs without a recording
// side (under Monitoring) or the output source's cable, unless chosen or
// Developer > Show every audio device is on (the source's cable then listed
// but disabled, saying so; issue #86: the list read as chaos). A device
// that has no profile yet shows what its first one gets: VB-CABLE, else the
// Hi-Fi Cable, on Auto (Off for LE Audio, which takes Auto as On; the
// core's default_mic_output). System Loopback's default output is not
// another device's Output Source its stream made the default. The
// Hi-Fi Cable with its two sides at different sample rates is said (it may
// pass nothing then; on one PC it still did). Off here and the microphone's
// button on the cards are one state (issue #86): a device whose mode is Auto
// or On is muted by Off and shows Off while muted, so the button can open it
// again in the same mode. Its recording side becomes Windows' default input
// while the device is connected unless turned off (the core's
// default_input.rs), so apps that record from the default microphone open it.

import { createSignal, Show } from "solid-js";
import { useApp } from "../app/context";
import { allAudioDevices, defaultDeviceSwitch, devicesSwitchShown } from "../app/settings";
import { t } from "../i18n";
import type { AudioDevice, Device, Profile } from "../ipc/types";
import {
  audioEntries,
  deviceKey,
  entryBase,
  HEADSET_MODES,
  headsetMode,
  hifiCable,
  isHifiCable,
  multichannelCable,
  ownDefaultOutput,
  type HeadsetMode,
  vbCable,
} from "../lib/profile";
import { saveHeadsetMic } from "../screens/Codecs";
import { Segmented, Select, Toggle, type Option } from "./controls";
import { Icon } from "./Icon";

/** The render endpoint the stream records with this source (the default
 *  output for System Loopback): the microphone must not go there */
function sourceDevice(
  source: Pick<Profile, "capture_mode" | "audio_device_id">,
  defaultOutput: string,
) {
  if (source.capture_mode === "virtual" || source.capture_mode === "output")
    return source.audio_device_id;
  if (source.capture_mode === "loopback") return defaultOutput;
  return "";
}

export function InputSource(props: {
  device: Device;
  devices: readonly AudioDevice[];
  /** The device's output source as shown (a loop is refused) */
  source: Pick<Profile, "capture_mode" | "audio_device_id"> & Partial<Pick<Profile, "auto_switch_device">>;
}) {
  const app = useApp();
  const snap = () => app.core.state.snap!;
  const entry = () =>
    audioEntries(snap()).find((e) => e.key === deviceKey(props.device)) ?? null;
  const all = () => allAudioDevices(snap().settings);
  // Not another device's Output Source made the default while it streams
  const defaultOutput = () => ownDefaultOutput(props.devices, snap());
  const looped = () => sourceDevice(props.source, defaultOutput());
  /** The profile shown: a classic device without one, its first one's defaults */
  const shown = (): Profile | null => {
    const e = entry();
    if (!e) return null;
    if (e.profile) return e.profile;
    const base = entryBase(e, snap().profiles);
    // VB-Cable first: the Hi-Fi Cable, with less delay, stays free for the sound
    const cable = [vbCable(props.devices), hifiCable(props.devices)].find(
      (c) => c && c.id !== looped(),
    );
    // LE Audio takes Auto as On: its microphone stays Off with the cable
    return cable
      ? {
          ...base,
          headset_mic: e.le_audio ? "off" : "auto",
          mic_device_id: cable.id,
          mic_device_name: cable.name,
        }
      : base;
  };
  /** The mode saved: what the microphone's button opens again */
  const saved = (): HeadsetMode => {
    const p = entry()?.profile;
    return p ? headsetMode(p) : "off";
  };
  /** The mode shown: Off while muted (the cards' button) */
  const mode = (): HeadsetMode => (!shown() || shown()!.mic_muted ? "off" : headsetMode(shown()!));
  /** Off mutes a device whose mode is Auto or On, as its button does */
  const chooseMode = (m: HeadsetMode) => {
    if (m === mode()) return;
    void save(
      m === "off" && saved() !== "off"
        ? { mic_muted: true }
        : { headset_mic: m, mic_muted: false },
    );
  };
  const chosen = () => shown()?.mic_device_id ?? "";
  /** The output source and what records with it (the other side of the
   *  same cable): the microphone would reach the headphones or be mixed
   *  with what plays, so these are listed but cannot be chosen */
  const usedBySource = (id: string) => {
    const source = props.devices.find((d) => d.id === looped());
    if (!source || !id) return false;
    const d = props.devices.find((x) => x.id === id);
    return (
      id === source.id ||
      (!!d?.capture_name && d.capture_name === source.capture_name)
    );
  };
  /** Windows plays its sound into the chosen device (issue #86: the
   *  microphone's cable left as the default output): apps hear it mixed with
   *  the microphone, and an Output Source that is another device gets
   *  nothing. Not while connecting makes the Output Source the default. */
  const playsDefault = () => {
    const d = props.devices.find((x) => x.id === chosen());
    const switches =
      props.source.capture_mode === "virtual" && defaultDeviceSwitch(snap().settings, props.source.auto_switch_device);
    return !!d && d.id === defaultOutput() && !switches && !usedBySource(d.id);
  };
  /** The Hi-Fi Cable chosen with its two sides at different sample rates:
   *  [playback, recording] */
  const hifiRates = (): [number, number] | null => {
    const d = props.devices.find((x) => x.id === chosen());
    if (!d || !isHifiCable(d) || !d.mix_rate || !d.capture_rate) return null;
    return d.mix_rate === d.capture_rate ? null : [d.mix_rate, d.capture_rate];
  };
  const [busy, setBusy] = createSignal(false);
  const save = async (patch: Partial<Profile>) => {
    const e = entry();
    const p = shown();
    if (!e || !p) return;
    setBusy(true);
    // The shown device goes with the first change (a device without a profile)
    await saveHeadsetMic(app, e, {
      mic_device_id: p.mic_device_id,
      mic_device_name: p.mic_device_name,
      ...patch,
    });
    setBusy(false);
  };
  const options = (): Option<string>[] => {
    // Cables first (VB-Cable, the microphone's own, then the Hi-Fi Cable and
    // the 16-channel one), then what only plays the microphone; a group's
    // options next to each other
    const rank = (d: AudioDevice) =>
      (d.capture_name || d.is_virtual ? 0 : 2) + Number(multichannelCable(d) || isHifiCable(d));
    const list = props.devices
      .filter(
        (d) =>
          d.id === chosen() ||
          all() ||
          (d.is_virtual && d.capture_name && !multichannelCable(d) && !usedBySource(d.id)),
      )
      .sort((a, b) => rank(a) - rank(b));
    // Outputs sharing a recording side ("CABLE Input" and "CABLE In 16ch" both
    // record as "CABLE Output"): only the first keeps that name, the others
    // add their own (the 16-channel one is after, so the standard cable keeps it)
    const label = (d: AudioDevice) => {
      if (!d.capture_name) return d.name;
      const first = list.find((x) => x.capture_name === d.capture_name);
      return first === d
        ? d.capture_name
        : `${d.capture_name} · ${d.name.replace(/\s*\(.*\)$/, "")}`;
    };
    const out: Option<string>[] = list.map((d) => ({
      value: d.id,
      label: usedBySource(d.id)
        ? t("capture.mic_used_by_source", label(d))
        : label(d),
      disabled: usedBySource(d.id),
      // Every output listed: those with no recording side only play the
      // microphone (issue #86: they read as microphones)
      group: all()
        ? d.capture_name || d.is_virtual
          ? t("capture.virtual")
          : t("capture.mic_monitoring")
        : undefined,
    }));
    const p = shown();
    // One chosen before and not there now: by its name
    if (p?.mic_device_id && !list.some((d) => d.id === p.mic_device_id)) {
      out.unshift({
        value: p.mic_device_id,
        label: t(
          "capture.volume_sync_missing",
          p.mic_device_name || p.mic_device_id,
        ),
      });
    }
    if (!p?.mic_device_id)
      out.unshift({ value: "", label: t("ui.codec.none"), disabled: true });
    return out;
  };
  const choose = (id: string) => {
    const d = props.devices.find((x) => x.id === id);
    if (!d || id === chosen()) return;
    void save({ mic_device_id: d.id, mic_device_name: d.name });
  };
  return (
    <Show when={shown()}>
      <div class="input-source">
        <span class="row-label">{t("capture.input_source")}</span>
        <div class="inline capture-source input-source-pick">
          <Icon name="mic" />
          <Select
            label={t("ui.mic_output")}
            value={chosen()}
            options={options()}
            disabled={busy() || mode() === "off"}
            onChange={choose}
          />
          <Segmented<HeadsetMode>
            label={t("capture.input_source")}
            value={mode()}
            options={HEADSET_MODES.map((m) => ({
              value: m,
              label: t(`ui.headset_mic_${m}`),
              disabled: busy(),
            }))}
            onChange={chooseMode}
          />
        </div>
        <p class="hint">{t("ui.headset_mic_quality")}</p>
        <Show when={mode() !== "off" && !chosen()}>
          <p class="field-warn">{t("ui.headset_mic_no_output")}</p>
        </Show>
        <Show when={mode() !== "off" && usedBySource(chosen())}>
          <p class="field-warn">
            <Icon name="warning" />
            <span>{t("capture.mic_loop_here")}</span>
          </p>
        </Show>
        <Show when={mode() !== "off" && hifiRates()}>
          {(r) => (
            <p class="field-warn">
              <Icon name="warning" />
              <span>{t("capture.hifi_rates_differ", r()[0], r()[1])}</span>
            </p>
          )}
        </Show>
        <Show when={mode() !== "off" && playsDefault()}>
          <p class="field-warn">
            <Icon name="warning" />
            <span>{t("capture.mic_is_default_output")}</span>
          </p>
        </Show>
        <Show when={devicesSwitchShown(snap().settings)}>
          <Toggle
            label={t("capture.mic_default_input")}
            checked={shown()?.mic_default_input !== false}
            disabled={busy()}
            onChange={(v) => void save({ mic_default_input: v })}
          />
        </Show>
      </div>
    </Show>
  );
}
