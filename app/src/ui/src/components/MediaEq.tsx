// A device's playback EQ, in a dialog from Audio Devices: on or off, and up to 16 points on
// a frequency-response graph (as Easy-MIC-Booster's): double-click adds a
// point (or puts a point's gain back to 0), dragging moves it (left/right
// the frequency on a log scale, up/down the gain), the wheel over it changes
// its Q, right-click changes its type or deletes it. The selected point's
// values are fields below the graph. A focused point moves with the arrow
// keys (Shift: bigger steps), Page Up / Page Down change its Q, Delete
// removes it. A new EQ starts with the classic ten flat bands
// (lib/eq.ts DEFAULT_POINTS), shown greyed out until it is turned on; Reset
// puts them back. Saved points are never replaced. Nothing hides (issue #86): while the EQ is off the graph,
// the fields and the buttons stay, disabled, and the fields are empty while
// no point is selected. Export saves the points as an Equalizer APO /
// AutoEq parametric EQ (lib/eq.ts eqApoText).
//
// While a point is dragged the stream plays it at once (media_eq.preview);
// the profile is saved when it is let go. The curve is the core's own
// filters computed here (lib/eq.ts); the core lowers the level by the
// highest boost and limits what still goes over, so the curve's shape is
// what changes, not the loudness.

import { createEffect, createMemo, createSignal, createUniqueId, For, onCleanup, Show } from "solid-js";
import { useApp } from "../app/context";
import { t } from "../i18n";
import type { Device, EqBandType, EqPoint, MediaEq } from "../ipc/types";
import {
  BAND_TYPES,
  DEFAULT_POINTS,
  eqApoText,
  FREQ_MAX,
  FREQ_MIN,
  GAIN_LIMIT,
  hasGain,
  hasQ,
  heldPoint,
  isDefaultPoints,
  MAX_POINTS,
  Q_MAX,
  Q_MIN,
  formatFreq,
  formatGain,
  responseCurve,
  roundFreq,
  roundGain,
  roundQ,
  withType,
} from "../lib/eq";
import { audioEntries, deviceKey } from "../lib/profile";
import { saveMediaEq } from "../screens/Codecs";
import { Button, Dialog, IconButton, NavRow, Select, Toggle, type Option } from "./controls";

const OFF: MediaEq = { enabled: false, bands: [] };
/** The exported file's suggested name */
const EXPORT_NAME = "BlueCaster EQ.txt";
/** Shortest time between two previews sent while dragging */
const PREVIEW_MS = 40;

const typeKey: Record<EqBandType, string> = {
  Peaking: "eq.type.peaking",
  LowShelf: "eq.type.low_shelf",
  HighShelf: "eq.type.high_shelf",
  HighPass: "eq.type.high_pass",
  LowPass: "eq.type.low_pass",
};
const typeLabel = (b: EqBandType) => t(typeKey[b]);

/** A line saying whether the playback EQ is on, with the editor in a dialog */
export function MediaEqSettings(props: { device: Device }) {
  const app = useApp();
  const [open, setOpen] = createSignal(false);
  const entry = () => audioEntries(app.core.state.snap!).find((e) => e.key === deviceKey(props.device)) ?? null;
  const on = () => entry()?.profile?.media_eq?.enabled === true;
  return (
    <>
      <NavRow
        label={t("eq.title")}
        value={t(on() ? "ui.state_on" : "ui.state_off")}
        onClick={() => setOpen(true)}
      />
      <Dialog open={open()} title={t("eq.title")} onCancel={() => setOpen(false)}>
        <MediaEqEditor device={props.device} />
      </Dialog>
    </>
  );
}

function MediaEqEditor(props: { device: Device }) {
  const app = useApp();
  const snap = () => app.core.state.snap!;
  const entry = () => audioEntries(snap()).find((e) => e.key === deviceKey(props.device)) ?? null;
  const saved = (): MediaEq => entry()?.profile?.media_eq ?? OFF;
  // What is shown while a change is on its way (dragged, or saved and not
  // back in the profile yet)
  const [draft, setDraft] = createSignal<MediaEq | null>(null);
  const eq = () => draft() ?? saved();
  const [selected, setSelected] = createSignal<number | null>(null);
  createEffect(() => {
    const d = draft();
    if (d && JSON.stringify(d) === JSON.stringify(saved())) setDraft(null);
  });
  createEffect(() => {
    const i = selected();
    if (i !== null && i >= eq().bands.length) setSelected(null);
  });

  // ---- live preview while dragging, saved when let go ----------------------
  let lastPreview = 0;
  let pending: number | undefined;
  onCleanup(() => clearTimeout(pending));
  const sendPreview = () => {
    pending = undefined;
    lastPreview = performance.now();
    const p = entry()?.profile;
    const d = draft();
    if (!p || !d || snap().stream.profile !== p.name || snap().stream.state === "idle") return;
    void app.core.request("media_eq.preview", { name: p.name, eq: d }).catch(() => {});
  };
  const preview = (next: MediaEq) => {
    setDraft(next);
    if (pending !== undefined) return;
    const wait = Math.max(0, PREVIEW_MS - (performance.now() - lastPreview));
    pending = window.setTimeout(sendPreview, wait);
  };
  const commit = async (next: MediaEq) => {
    clearTimeout(pending);
    pending = undefined;
    setDraft(next);
    const e = entry();
    if (!e || !(await saveMediaEq(app, e, next))) setDraft(null);
  };

  const setBands = (bands: EqPoint[], done: boolean) => {
    const next = { ...eq(), bands };
    if (done) void commit(next);
    else preview(next);
  };
  const editSelected = (patch: Partial<EqPoint>) => {
    const i = selected();
    if (i === null) return;
    const bands = eq().bands.slice();
    bands[i] = heldPoint({ ...bands[i], ...patch });
    setBands(bands, true);
  };
  const addPoint = () => {
    const bands = eq().bands;
    if (bands.length >= MAX_POINTS) return;
    // Where no point is yet, from 1 kHz outwards
    const taken = (f: number) => bands.some((b) => Math.abs(Math.log2(b.frequency / f)) < 0.5);
    // Then between the default bands
    const free =
      [1000, 2000, 500, 4000, 250, 8000, 125, 16000, 63, 31, 1400, 700, 2800, 350, 5600, 180, 11000, 90, 45].find(
        (f) => !taken(f),
      ) ?? 1000;
    setBands([...bands, { frequency: free, gain: 0, q: 1, band_type: "Peaking" }], true);
    setSelected(bands.length);
  };
  const sel = () => {
    const i = selected();
    return i === null ? null : (eq().bands[i] ?? null);
  };
  // Editing: on, with a profile to save it in; off, everything stays but disabled
  const active = () => eq().enabled && !!entry();
  // Off without points: the ones turning it on gives
  const shownBands = () => (!eq().enabled && eq().bands.length === 0 ? DEFAULT_POINTS : eq().bands);
  createEffect(() => {
    if (!active()) setSelected(null);
  });
  const deleteSelected = () => {
    const i = selected();
    if (i === null) return;
    setSelected(null);
    setBands(
      eq().bands.filter((_, j) => j !== i),
      true,
    );
  };
  const [exportError, setExportError] = createSignal<string | null>(null);
  const exportPoints = async () => {
    setExportError(null);
    try {
      await app.core.transport.saveTextFile(EXPORT_NAME, eqApoText(eq().bands));
    } catch (e) {
      setExportError(t("eq.export_failed", String(e)));
    }
  };
  const pointId = createUniqueId();
  const typeId = createUniqueId();
  // While none is selected, a blank first entry: the box shows nothing
  const pointOptions = (): Option<number>[] => [
    ...(selected() === null ? [{ value: -1, label: "", disabled: true }] : []),
    ...eq().bands.map((_, i) => ({ value: i, label: String(i + 1) })),
  ];
  const typeOptions = (): Option<EqBandType>[] =>
    sel() ? BAND_TYPES.map((b) => ({ value: b, label: typeLabel(b) })) : [];

  return (
    <div class="media-eq">
      <Toggle
        label={t("eq.enable")}
        checked={eq().enabled}
        disabled={!entry()}
        onChange={(v) =>
          // A new EQ starts with the default points
          void commit({ enabled: v, bands: v && eq().bands.length === 0 ? [...DEFAULT_POINTS] : eq().bands })
        }
      />
      <EqGraph
        bands={shownBands()}
        selected={selected()}
        disabled={!active()}
        onSelect={setSelected}
        onChange={setBands}
      />
      <p class="hint">{t("eq.graph_hint")}</p>
      <div class="eq-fields">
        <div class="eq-field eq-point-field">
          <label for={pointId}>{t("eq.point")}</label>
          <span class="eq-point-pick">
            <IconButton
              icon="delete"
              label={t("eq.delete_point")}
              disabled={!active() || !sel()}
              onClick={deleteSelected}
            />
            <Select
              id={pointId}
              value={selected() ?? -1}
              options={pointOptions()}
              disabled={!active() || eq().bands.length === 0}
              onChange={(i) => setSelected(i)}
            />
          </span>
        </div>
        <div class="eq-field">
          <label for={typeId}>{t("eq.type")}</label>
          <Select
            id={typeId}
            value={sel()?.band_type ?? "Peaking"}
            options={typeOptions()}
            disabled={!active() || !sel()}
            onChange={(v) => {
              const b = sel();
              if (b) editSelected(withType(b, v));
            }}
          />
        </div>
        <NumberField
          label={t("eq.frequency")}
          unit="Hz"
          wide
          value={sel()?.frequency ?? null}
          min={FREQ_MIN}
          max={FREQ_MAX}
          step={1}
          disabled={!active()}
          onChange={(v) => editSelected({ frequency: v })}
        />
        <NumberField
          label={t("eq.gain")}
          unit="dB"
          value={sel()?.gain ?? null}
          min={-GAIN_LIMIT}
          max={GAIN_LIMIT}
          step={0.1}
          disabled={!active() || !hasGain(sel()?.band_type ?? "Peaking")}
          onChange={(v) => editSelected({ gain: v })}
        />
        <NumberField
          label="Q"
          value={sel()?.q ?? null}
          min={Q_MIN}
          max={Q_MAX}
          step={0.01}
          disabled={!active() || !hasQ(sel()?.band_type ?? "Peaking")}
          onChange={(v) => editSelected({ q: v })}
        />
      </div>
      <div class="eq-actions">
        <Button icon="add" disabled={!active() || eq().bands.length >= MAX_POINTS} onClick={addPoint}>
          {t("eq.add_point")}
        </Button>
        <Button
          icon="arrow_reset"
          disabled={!active() || isDefaultPoints(eq().bands)}
          onClick={() => {
            setSelected(null);
            setBands([...DEFAULT_POINTS], true);
          }}
        >
          {t("eq.reset")}
        </Button>
        <Button
          icon="arrow_download"
          disabled={!active() || eq().bands.length === 0}
          onClick={() => void exportPoints()}
        >
          {t("eq.export")}
        </Button>
      </div>
      <Show when={exportError()}>{(e) => <p class="field-warn">{e()}</p>}</Show>
    </div>
  );
}

/** A number typed in, taken when it is left or Enter is pressed; empty
 *  (and disabled) while there is no value */
function NumberField(props: {
  label: string;
  unit?: string;
  /** As wide as the Type box (five digits) */
  wide?: boolean;
  value: number | null;
  min: number;
  max: number;
  step: number;
  disabled?: boolean;
  onChange: (v: number) => void;
}) {
  const id = createUniqueId();
  const off = () => props.disabled || props.value === null;
  return (
    <div class="eq-field">
      <label for={id}>{props.label}</label>
      <span class="eq-number">
        <input
          id={id}
          class="input num"
          classList={{ wide: props.wide }}
          type="number"
          min={props.min}
          max={props.max}
          step={props.step}
          disabled={off()}
          value={off() ? "" : String(props.value)}
          onChange={(e) => {
            const v = Number(e.currentTarget.value);
            if (props.value === null) return;
            if (e.currentTarget.value === "" || !Number.isFinite(v)) {
              e.currentTarget.value = String(props.value);
              return;
            }
            props.onChange(Math.min(props.max, Math.max(props.min, v)));
          }}
        />
        <Show when={props.unit}>
          <span class="eq-unit">{props.unit}</span>
        </Show>
      </span>
    </div>
  );
}

// ---- the graph --------------------------------------------------------------

const W = 640;
const H = 240;
const PLOT = { left: 38, right: W - 10, top: 10, bottom: H - 22 };
const DB_RANGE = GAIN_LIMIT;
const GRID_F = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
const GRID_DB = [-12, -9, -6, -3, 0, 3, 6, 9, 12];
const LOG_SPAN = Math.log10(FREQ_MAX / FREQ_MIN);

const xOf = (f: number) =>
  PLOT.left + (Math.log10(Math.min(FREQ_MAX, Math.max(FREQ_MIN, f)) / FREQ_MIN) / LOG_SPAN) * (PLOT.right - PLOT.left);
const fOf = (x: number) =>
  FREQ_MIN * Math.pow(10, Math.min(1, Math.max(0, (x - PLOT.left) / (PLOT.right - PLOT.left))) * LOG_SPAN);
const yOf = (db: number) => PLOT.top + ((DB_RANGE - db) / (2 * DB_RANGE)) * (PLOT.bottom - PLOT.top);
const dbOf = (y: number) => DB_RANGE - ((y - PLOT.top) / (PLOT.bottom - PLOT.top)) * 2 * DB_RANGE;

type Menu = { x: number; y: number; point: number | null };

export function EqGraph(props: {
  bands: readonly EqPoint[];
  selected: number | null;
  /** Shown greyed out, nothing on it reacts (the EQ is off) */
  disabled?: boolean;
  onSelect: (i: number | null) => void;
  /** New points; `done`: the change is complete (save it), else it is
   *  being dragged */
  onChange: (bands: EqPoint[], done: boolean) => void;
}) {
  let svg!: SVGSVGElement;
  const [drag, setDrag] = createSignal<number | null>(null);
  const [hover, setHover] = createSignal<number | null>(null);
  const [menu, setMenu] = createSignal<Menu | null>(null);
  // The points while dragging (the last move), saved when let go
  let dragged: EqPoint[] | null = null;

  const curve = createMemo(() => {
    const pts = responseCurve(props.bands, 220);
    const clampY = (db: number) => Math.min(PLOT.bottom + 40, Math.max(PLOT.top - 40, yOf(db)));
    const line = pts.map((p, i) => `${i ? "L" : "M"}${xOf(p.f).toFixed(1)},${clampY(p.db).toFixed(1)}`).join("");
    const zero = yOf(0).toFixed(1);
    return { line, fill: `${line}L${PLOT.right},${zero}L${PLOT.left},${zero}Z` };
  });

  /** A pointer event's position in the graph's coordinates */
  const at = (e: { clientX: number; clientY: number }) => {
    const r = svg.getBoundingClientRect();
    return { x: ((e.clientX - r.left) * W) / r.width, y: ((e.clientY - r.top) * H) / r.height };
  };
  const pointY = (b: EqPoint) => yOf(hasGain(b.band_type) ? b.gain : 0);

  const replace = (i: number, b: EqPoint) => {
    const bands = props.bands.slice();
    bands[i] = heldPoint(b);
    return bands;
  };
  const addAt = (x: number, y: number) => {
    if (props.bands.length >= MAX_POINTS || !Number.isFinite(x) || !Number.isFinite(y)) return;
    const p: EqPoint = { frequency: roundFreq(fOf(x)), gain: roundGain(dbOf(y)), q: 1, band_type: "Peaking" };
    const i = props.bands.length;
    props.onChange([...props.bands, p], true);
    props.onSelect(i);
  };

  const onPointDown = (i: number, e: PointerEvent) => {
    if (e.button !== 0 || props.disabled) return;
    e.stopPropagation();
    setMenu(null);
    props.onSelect(i);
    setDrag(i);
    dragged = null;
    svg.setPointerCapture(e.pointerId);
  };
  const onMove = (e: PointerEvent) => {
    const i = drag();
    if (i === null) return;
    const { x, y } = at(e);
    const b = props.bands[i];
    if (!b || !Number.isFinite(x) || !Number.isFinite(y)) return;
    const moved: EqPoint = {
      ...b,
      frequency: roundFreq(fOf(x)),
      gain: hasGain(b.band_type) ? roundGain(dbOf(y)) : 0,
    };
    if (moved.frequency === b.frequency && moved.gain === b.gain) return;
    dragged = replace(i, moved);
    props.onChange(dragged, false);
  };
  const onUp = (e: PointerEvent) => {
    if (drag() === null) return;
    setDrag(null);
    if (svg.hasPointerCapture(e.pointerId)) svg.releasePointerCapture(e.pointerId);
    if (dragged) props.onChange(dragged, true);
    dragged = null;
  };
  const onWheel = (i: number, e: WheelEvent) => {
    const b = props.bands[i];
    if (!b || !hasQ(b.band_type) || props.disabled) return;
    e.preventDefault();
    const q = roundQ(b.q * Math.pow(1.0015, -e.deltaY));
    if (q !== b.q) props.onChange(replace(i, { ...b, q }), true);
  };
  const onKey = (i: number, e: KeyboardEvent) => {
    const b = props.bands[i];
    if (!b || props.disabled) return;
    const big = e.shiftKey;
    let next: EqPoint | null = null;
    switch (e.key) {
      case "ArrowLeft":
      case "ArrowRight": {
        // 1/12 octave, an octave with Shift
        const step = (big ? 1 : 1 / 12) * (e.key === "ArrowLeft" ? -1 : 1);
        next = { ...b, frequency: roundFreq(b.frequency * Math.pow(2, step)) };
        if (next.frequency === b.frequency) next.frequency = b.frequency + (step > 0 ? 1 : -1);
        break;
      }
      case "ArrowUp":
      case "ArrowDown":
        if (!hasGain(b.band_type)) return;
        next = { ...b, gain: roundGain(b.gain + (big ? 3 : 0.5) * (e.key === "ArrowUp" ? 1 : -1)) };
        break;
      case "PageUp":
      case "PageDown":
        if (!hasQ(b.band_type)) return;
        next = { ...b, q: roundQ(b.q * (e.key === "PageUp" ? 1.25 : 0.8)) };
        break;
      case "Delete":
      case "Backspace":
        e.preventDefault();
        props.onSelect(null);
        props.onChange(
          props.bands.filter((_, j) => j !== i),
          true,
        );
        return;
      case "ContextMenu":
        e.preventDefault();
        setMenu({ x: xOf(b.frequency), y: pointY(b), point: i });
        return;
      default:
        return;
    }
    e.preventDefault();
    props.onChange(replace(i, next), true);
  };

  const pointLabel = (b: EqPoint, i: number) =>
    t(
      "eq.point_label",
      i + 1,
      [typeLabel(b.band_type), formatFreq(b.frequency), hasGain(b.band_type) ? formatGain(b.gain) : "", hasQ(b.band_type) ? `Q ${b.q.toFixed(2)}` : ""]
        .filter(Boolean)
        .join(", "),
    );
  // The hovered or dragged point's values (not over its menu)
  const shown = () => {
    if (menu() || props.disabled) return null;
    const i = drag() ?? hover();
    return i === null ? null : (props.bands[i] ? { i, b: props.bands[i] } : null);
  };

  // A menu closes on a click elsewhere or Escape
  const closeMenu = (e: Event) => {
    if (e instanceof KeyboardEvent && e.key !== "Escape") return;
    if (e.type === "pointerdown" && (e.target as Element | null)?.closest?.(".eq-menu")) return;
    setMenu(null);
  };
  createEffect(() => {
    if (props.disabled) {
      setMenu(null);
      setDrag(null);
      dragged = null;
    }
  });
  createEffect(() => {
    if (!menu()) return;
    document.addEventListener("pointerdown", closeMenu, true);
    document.addEventListener("keydown", closeMenu, true);
    onCleanup(() => {
      document.removeEventListener("pointerdown", closeMenu, true);
      document.removeEventListener("keydown", closeMenu, true);
    });
  });
  const menuItem = (fn: () => void) => () => {
    fn();
    setMenu(null);
  };

  return (
    <div class="eq-graph" classList={{ disabled: props.disabled }}>
      <svg
        ref={svg}
        viewBox={`0 0 ${W} ${H}`}
        role="group"
        aria-label={t("eq.graph_label")}
        aria-disabled={props.disabled ? "true" : undefined}
        classList={{ dragging: drag() !== null }}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onPointerDown={(e) => {
          if (props.disabled) return;
          if (e.button === 0) {
            setMenu(null);
            props.onSelect(null);
          }
        }}
        onDblClick={(e) => {
          if (props.disabled) return;
          const { x, y } = at(e);
          addAt(x, y);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          if (props.disabled) return;
          const { x, y } = at(e);
          setMenu({ x, y, point: null });
        }}
      >
        <defs>
          <clipPath id="eq-plot-clip">
            <rect x={PLOT.left} y={PLOT.top} width={PLOT.right - PLOT.left} height={PLOT.bottom - PLOT.top} />
          </clipPath>
        </defs>
        <rect class="eq-bg" x={PLOT.left} y={PLOT.top} width={PLOT.right - PLOT.left} height={PLOT.bottom - PLOT.top} />
        <For each={GRID_F}>
          {(f) => (
            <>
              <line class="eq-grid" x1={xOf(f)} x2={xOf(f)} y1={PLOT.top} y2={PLOT.bottom} />
              <text class="eq-axis" x={xOf(f)} y={H - 6} text-anchor="middle">
                {f >= 1000 ? `${f / 1000}k` : f}
              </text>
            </>
          )}
        </For>
        <For each={GRID_DB}>
          {(db) => (
            <>
              <line class="eq-grid" classList={{ zero: db === 0 }} x1={PLOT.left} x2={PLOT.right} y1={yOf(db)} y2={yOf(db)} />
              <text class="eq-axis" x={PLOT.left - 6} y={yOf(db) + 4} text-anchor="end">
                {db > 0 ? `+${db}` : db < 0 ? `−${-db}` : "0"}
              </text>
            </>
          )}
        </For>
        <g clip-path="url(#eq-plot-clip)">
          <path class="eq-fill" d={curve().fill} />
          <path class="eq-curve" d={curve().line} />
        </g>
        <For each={props.bands}>
          {(b, i) => (
            <g
              class="eq-point"
              classList={{ on: props.selected === i() }}
              transform={`translate(${xOf(b.frequency).toFixed(1)},${pointY(b).toFixed(1)})`}
              tabindex={props.disabled ? -1 : 0}
              role="button"
              aria-label={pointLabel(b, i())}
              aria-pressed={props.selected === i()}
              aria-disabled={props.disabled ? "true" : undefined}
              onFocus={() => {
                if (!props.disabled) props.onSelect(i());
              }}
              onPointerDown={(e) => onPointDown(i(), e)}
              onPointerEnter={() => setHover(i())}
              onPointerLeave={() => setHover((h) => (h === i() ? null : h))}
              onDblClick={(e) => {
                e.stopPropagation();
                if (props.disabled) return;
                if (hasGain(b.band_type) && b.gain !== 0) props.onChange(replace(i(), { ...b, gain: 0 }), true);
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                if (props.disabled) return;
                props.onSelect(i());
                setMenu({ x: xOf(b.frequency), y: pointY(b), point: i() });
              }}
              onWheel={(e) => onWheel(i(), e)}
              onKeyDown={(e) => onKey(i(), e)}
            >
              <circle class="eq-halo" r="14" />
              <circle class="eq-dot" r="9" />
              <text class="eq-num" y="4" text-anchor="middle">
                {i() + 1}
              </text>
            </g>
          )}
        </For>
      </svg>
      <Show when={shown()}>
        {(s) => (
          <div class="eq-tip num" style={tipStyle(xOf(s().b.frequency), pointY(s().b))} aria-hidden="true">
            {[
              typeLabel(s().b.band_type),
              formatFreq(s().b.frequency),
              hasGain(s().b.band_type) ? formatGain(s().b.gain) : "",
              hasQ(s().b.band_type) ? `Q ${s().b.q.toFixed(2)}` : "",
            ]
              .filter(Boolean)
              .join(" · ")}
          </div>
        )}
      </Show>
      <Show when={menu()}>
        {(m) => (
          <div
            class="eq-menu"
            role="menu"
            style={menuStyle(m().x, m().y)}
            ref={(el) => queueMicrotask(() => el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus())}
          >
            <Show
              when={m().point !== null && props.bands[m().point!]}
              fallback={
                <>
                  <button
                    type="button"
                    role="menuitem"
                    disabled={props.bands.length >= MAX_POINTS}
                    onClick={menuItem(() => addAt(m().x, m().y))}
                  >
                    {t("eq.add_point_here")}
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    disabled={isDefaultPoints(props.bands)}
                    onClick={menuItem(() => {
                      props.onSelect(null);
                      props.onChange([...DEFAULT_POINTS], true);
                    })}
                  >
                    {t("eq.reset")}
                  </button>
                </>
              }
            >
              {(b) => (
                <>
                  <For each={BAND_TYPES}>
                    {(kind) => (
                      <button
                        type="button"
                        role="menuitemradio"
                        aria-checked={b().band_type === kind}
                        onClick={menuItem(() => props.onChange(replace(m().point!, withType(b(), kind)), true))}
                      >
                        {typeLabel(kind)}
                      </button>
                    )}
                  </For>
                  <hr />
                  <button
                    type="button"
                    role="menuitem"
                    onClick={menuItem(() => {
                      const i = m().point!;
                      props.onSelect(null);
                      props.onChange(
                        props.bands.filter((_, j) => j !== i),
                        true,
                      );
                    })}
                  >
                    {t("eq.delete_point")}
                  </button>
                </>
              )}
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

/** A position in the graph's coordinates as CSS, relative to its box */
const pct = (x: number, y: number) => ({ left: `${(x / W) * 100}%`, top: `${(y / H) * 100}%` });
const tipStyle = (x: number, y: number) => {
  const p = pct(x, y);
  return { left: p.left, top: p.top, transform: `translate(${x > W * 0.7 ? "calc(-100% - 14px)" : "14px"}, ${y < 40 ? "12px" : "calc(-100% - 12px)"})` };
};
const menuStyle = (x: number, y: number) => {
  const p = pct(x, y);
  return { left: p.left, top: p.top, transform: `translate(${x > W * 0.7 ? "-100%" : "0"}, ${y > H * 0.5 ? "-100%" : "0"})` };
};
