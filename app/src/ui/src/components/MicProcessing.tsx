// The headset microphone's processing, in a dialog opened under its Input
// Source: a switch over all of it (off keeps the values, greyed out), then
// gain, noise suppression's strength and the noise gate's threshold (each
// at its lowest: off). Kept in the device's profile; the running
// microphone takes a change in place, nothing reconnects. A slider saves
// when let go and shows what it holds until the profile has it. Nothing
// hides (issue #86): while the microphone is Off the line and the controls
// stay, disabled.

import { createSignal, createUniqueId, Show } from "solid-js";
import { t } from "../i18n";
import type { MicProcessing } from "../ipc/types";
import { Dialog, NavRow, Toggle } from "./controls";

/** The switch over the processing is on */
export const micProcessingOn = (p: MicProcessing) => p.enabled;

/** A line saying whether the processing is on, with the dialog behind it
 *  (`off`: the microphone is Off, the line disabled; `busy`: a change is
 *  on its way) */
export function MicProcessingRow(props: {
  value: MicProcessing;
  off?: boolean;
  busy?: boolean;
  onChange: (p: MicProcessing) => Promise<unknown>;
}) {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <NavRow
        label={t("ui.mic_processing")}
        value={t(micProcessingOn(props.value) ? "ui.state_on" : "ui.state_off")}
        disabled={props.off}
        onClick={() => setOpen(true)}
      />
      <Dialog open={open()} title={t("ui.mic_processing")} onCancel={() => setOpen(false)}>
        <MicProcessingControls value={props.value} disabled={props.off || props.busy} onChange={props.onChange} />
      </Dialog>
    </>
  );
}

export const MIC_GAIN_RANGE = [-12, 24] as const;
export const MIC_GATE_RANGE = [-80, -20] as const;

export const gainText = (db: number) => `${db > 0 ? "+" : ""}${db} dB`;
export const gateText = (db: number) => (db <= MIC_GATE_RANGE[0] ? t("ui.mic_gate_off") : `${db} dB`);
export const suppressionText = (v: number) => (v <= 0 ? t("ui.mic_suppression_off") : `${v}%`);

function Slider(props: {
  label: string;
  /** A line under the label: what the setting does */
  hint?: string;
  min: number;
  max: number;
  value: number;
  text: (v: number) => string;
  disabled?: boolean;
  onSave: (v: number) => Promise<unknown>;
}) {
  const id = createUniqueId();
  // Being dragged, then saved and not back from the core yet
  const [held, setHeld] = createSignal<number | null>(null);
  const [pending, setPending] = createSignal<number | null>(null);
  const shown = () => held() ?? pending() ?? props.value;
  const fill = () => ((shown() - props.min) / (props.max - props.min)) * 100;
  const letGo = () => {
    const v = held();
    setHeld(null);
    if (v === null || v === props.value) return;
    setPending(v);
    void props.onSave(v).finally(() => setPending(null));
  };
  return (
    <div class="row mic-slider">
      <label class="row-text" for={id}>
        <span class="row-label" id={`${id}-label`}>
          {props.label}
        </span>
        <Show when={props.hint}>
          <span class="hint" id={`${id}-hint`}>
            {props.hint}
          </span>
        </Show>
      </label>
      <div class="volume">
        <input
          id={id}
          type="range"
          min={props.min}
          max={props.max}
          step="1"
          value={shown()}
          disabled={props.disabled}
          aria-valuetext={props.text(shown())}
          aria-labelledby={`${id}-label`}
          aria-describedby={props.hint ? `${id}-hint` : undefined}
          style={{ "--fill": `${fill()}%` }}
          onInput={(e) => setHeld(e.currentTarget.valueAsNumber)}
          onChange={letGo}
        />
        <output class="volume-value num" for={id}>
          {props.text(shown())}
        </output>
      </div>
    </div>
  );
}

export function MicProcessingControls(props: {
  value: MicProcessing;
  disabled?: boolean;
  onChange: (p: MicProcessing) => Promise<unknown>;
}) {
  const save = (patch: Partial<MicProcessing>) => props.onChange({ ...props.value, ...patch });
  const sliderOff = () => props.disabled || !props.value.enabled;
  return (
    <div class="mic-processing">
      <Toggle
        label={t("ui.mic_processing_enable")}
        checked={props.value.enabled}
        disabled={props.disabled}
        onChange={(v) => void save({ enabled: v })}
      />
      <Slider
        label={t("ui.mic_gain")}
        min={MIC_GAIN_RANGE[0]}
        max={MIC_GAIN_RANGE[1]}
        value={props.value.gain_db}
        text={gainText}
        disabled={sliderOff()}
        onSave={(v) => save({ gain_db: v })}
      />
      <Slider
        label={t("ui.mic_noise_suppression")}
        min={0}
        max={100}
        value={props.value.suppression}
        text={suppressionText}
        disabled={sliderOff()}
        onSave={(v) => save({ suppression: v })}
      />
      <Slider
        label={t("ui.mic_gate")}
        hint={t("ui.mic_gate_hint")}
        min={MIC_GATE_RANGE[0]}
        max={MIC_GATE_RANGE[1]}
        value={Math.max(props.value.gate_db, MIC_GATE_RANGE[0])}
        text={gateText}
        disabled={sliderOff()}
        onSave={(v) => save({ gate_db: v })}
      />
    </div>
  );
}
