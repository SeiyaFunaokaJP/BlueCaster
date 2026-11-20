// The playback EQ on Audio Devices: on and off, points added on the graph
// (double-click, the button), moved by keyboard and by dragging (tried out
// on the stream while dragged, saved when let go), their type changed from
// the right-click menu, picked and edited in the fields, reset and exported.
// Nothing hides: off, the graph, the fields and the buttons stay, disabled.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "../app/App";
import { setLanguage, t } from "../i18n";
import { MockCore } from "../ipc/mock";
import type { MediaEq, Profile } from "../ipc/types";
import { createCore } from "../ipc/store";
import { DEFAULT_POINTS } from "../lib/eq";
import { endSettle } from "./Codecs";

let mock: MockCore | null = null;

function start() {
  location.hash = "#/devices";
  const calls: { method: string; params: unknown }[] = [];
  mock = new MockCore({ scenario: "normal", speed: 0, onCall: (method, params) => calls.push({ method, params }) });
  const core = createCore(mock);
  render(() => <App core={core} />);
  const saved = () =>
    calls
      .filter((c) => c.method === "profile.save")
      .map((c) => (c.params as { profile: Profile }).profile.media_eq as MediaEq);
  const lastEq = () => saved()[saved().length - 1];
  return { calls, core, saved, lastEq };
}

afterEach(async () => {
  cleanup();
  endSettle();
  mock?.dispose();
  mock = null;
  await setLanguage("en");
});

const deviceEntry = (name: RegExp) =>
  waitFor(() => {
    const entry = [...document.querySelectorAll<HTMLElement>("main .device-entry")].find((e) =>
      name.test(e.textContent ?? ""),
    );
    expect(entry).toBeTruthy();
    return entry!;
  });

/** The graph's box as a browser lays it out: 640 x 240 px at (0, 0) */
function layOut(svg: SVGSVGElement) {
  svg.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: 640, height: 240, right: 640, bottom: 240, x: 0, y: 0, toJSON() {} }) as DOMRect;
  svg.setPointerCapture = () => {};
  svg.releasePointerCapture = () => {};
  svg.hasPointerCapture = () => true;
}

/** The EQ turned on (with the default points; `empty`: then deleted) */
async function eqOn(empty = false) {
  const env = start();
  const entry = await deviceEntry(/WH-1000XM5/);
  // A line on the device, the EQ itself in a dialog
  const row = within(entry).getByRole("button", { name: new RegExp(`^${t("eq.title")}`) });
  expect(row).toHaveTextContent(t("ui.state_off"));
  expect(entry.querySelector(".media-eq")).toBeNull();
  fireEvent.click(row);
  const wh = await screen.findByRole("dialog", { name: t("eq.title") });
  const toggle = within(wh).getByRole("switch", { name: t("eq.enable") });
  expect(toggle).not.toBeChecked();
  // Off: shown, but disabled, with the points turning it on gives
  expect(wh.querySelector(".eq-graph.disabled")).toBeTruthy();
  expect(within(wh).getAllByRole("button", { name: /^Point \d+:/ })).toHaveLength(10);
  expect(within(wh).getByRole("button", { name: t("eq.add_point") })).toBeDisabled();
  expect(within(wh).getByRole("spinbutton", { name: t("eq.frequency") })).toBeDisabled();
  fireEvent.click(toggle);
  await waitFor(() => expect(env.lastEq()).toEqual({ enabled: true, bands: DEFAULT_POINTS }));
  await waitFor(() => expect(row).toHaveTextContent(t("ui.state_on")));
  const svg = await waitFor(() => {
    const s = wh.querySelector<SVGSVGElement>(".eq-graph:not(.disabled) svg");
    expect(s).toBeTruthy();
    return s!;
  });
  layOut(svg);
  // The core's profile has the last save (a change made before it arrives
  // starts from the profile as it was)
  const synced = () =>
    waitFor(() =>
      expect(env.core.state.snap!.profiles.find((p) => p.device_name === "WH-1000XM5")?.media_eq).toEqual(
        env.lastEq(),
      ),
    );
  if (empty) {
    for (let n = DEFAULT_POINTS.length; n > 0; n--) {
      await synced();
      fireEvent.keyDown(within(wh).getByRole("button", { name: /^Point 1:/ }), { key: "Delete" });
      await waitFor(() => expect(env.lastEq().bands).toHaveLength(n - 1));
    }
    await synced();
  }
  return { ...env, wh, svg, synced };
}

describe("Playback EQ", () => {
  it("is off until turned on, and adds a point where the graph is double-clicked", async () => {
    const { wh, svg, lastEq } = await eqOn(true);
    // 1 kHz is 2/3 of the way along the log axis; +6 dB a quarter from the top
    fireEvent.dblClick(svg, { clientX: 38 + (592 * Math.log10(50)) / 3, clientY: 10 + 208 * 0.25 });
    await waitFor(() => expect(lastEq().bands).toHaveLength(1));
    const p = lastEq().bands[0];
    expect(p.band_type).toBe("Peaking");
    expect(p.frequency).toBe(1000);
    expect(p.gain).toBeCloseTo(6, 1);
    const point = await within(wh).findByRole("button", { name: /^Point 1: Peak, 1.00 kHz, \+6.0 dB, Q 1.00$/ });
    expect(point).toHaveAttribute("aria-pressed", "true");
    // The selected point's fields
    expect(within(wh).getByRole("spinbutton", { name: t("eq.frequency") })).toHaveValue(1000);
  });

  it("moves a focused point with the keyboard and deletes it", async () => {
    const { wh, lastEq, synced } = await eqOn(true);
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    const point = await within(wh).findByRole("button", { name: /^Point 1:/ });
    point.focus();
    fireEvent.keyDown(point, { key: "ArrowUp" });
    await waitFor(() => expect(lastEq().bands[0].gain).toBe(0.5));
    fireEvent.keyDown(within(wh).getByRole("button", { name: /^Point 1:/ }), { key: "ArrowRight", shiftKey: true });
    await waitFor(() => expect(lastEq().bands[0].frequency).toBe(2000));
    fireEvent.keyDown(within(wh).getByRole("button", { name: /^Point 1:/ }), { key: "PageUp" });
    await waitFor(() => expect(lastEq().bands[0].q).toBe(1.25));
    await synced();
    fireEvent.keyDown(within(wh).getByRole("button", { name: /^Point 1:/ }), { key: "Delete" });
    await waitFor(() => expect(lastEq().bands).toEqual([]));
  });

  it("changes a point's type from its menu, and resets", async () => {
    const { wh, lastEq, synced } = await eqOn(true);
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    const point = await within(wh).findByRole("button", { name: /^Point 1:/ });
    fireEvent.keyDown(point, { key: "ArrowUp", shiftKey: true });
    await waitFor(() => expect(lastEq().bands[0].gain).toBe(3));
    fireEvent.contextMenu(within(wh).getByRole("button", { name: /^Point 1:/ }));
    const menu = await within(wh).findByRole("menu");
    fireEvent.click(within(menu).getByRole("menuitemradio", { name: t("eq.type.high_pass") }));
    await waitFor(() => expect(lastEq().bands[0]).toMatchObject({ band_type: "HighPass", gain: 0 }));
    expect(within(wh).queryByRole("menu")).toBeNull();
    // A pass filter has no gain to type
    expect(within(wh).getByRole("spinbutton", { name: t("eq.gain") })).toBeDisabled();
    expect(within(wh).getByRole("combobox", { name: t("eq.type") })).toHaveValue("HighPass");
    await synced();
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.reset") }));
    await waitFor(() => expect(lastEq()).toEqual({ enabled: true, bands: DEFAULT_POINTS }));
  });

  it("starts with ten flat points, which Reset puts back and Export writes", async () => {
    const { wh, lastEq, synced } = await eqOn();
    const reset = within(wh).getByRole("button", { name: t("eq.reset") });
    expect(reset).toBeDisabled();
    fireEvent.keyDown(within(wh).getByRole("button", { name: /^Point 6:/ }), { key: "ArrowUp" });
    await waitFor(() => expect(lastEq().bands[5]).toMatchObject({ frequency: 1000, gain: 0.5 }));
    await synced();
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.export") }));
    await waitFor(() => expect(mock!.savedFiles).toHaveLength(1));
    const lines = mock!.savedFiles[0].text.split(String.fromCharCode(13, 10));
    expect(lines).toHaveLength(12);
    expect(lines[1]).toBe("Filter 1: ON PK Fc 31 Hz Gain 0.0 dB Q 1.41");
    expect(lines[6]).toBe("Filter 6: ON PK Fc 1000 Hz Gain 0.5 dB Q 1.41");
    expect(lines[10]).toBe("Filter 10: ON PK Fc 16000 Hz Gain 0.0 dB Q 1.41");
    expect(reset).toBeEnabled();
    fireEvent.click(reset);
    await waitFor(() => expect(lastEq().bands).toEqual(DEFAULT_POINTS));
  });

  it("takes typed values held to their range", async () => {
    const { wh, lastEq } = await eqOn(true);
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    await within(wh).findByRole("button", { name: /^Point 1:/ });
    const gain = within(wh).getByRole("spinbutton", { name: t("eq.gain") });
    fireEvent.change(gain, { target: { value: "40" } });
    await waitFor(() => expect(lastEq().bands[0].gain).toBe(12));
    fireEvent.change(within(wh).getByRole("spinbutton", { name: "Q" }), { target: { value: "2.5" } });
    await waitFor(() => expect(lastEq().bands[0].q).toBe(2.5));
  });

  it("saves a dragged point once it is let go", async () => {
    const { wh, svg, saved, lastEq } = await eqOn(true);
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    const point = await within(wh).findByRole("button", { name: /^Point 1:/ });
    const before = saved().length;
    fireEvent.pointerDown(point, { button: 0, pointerId: 1, clientX: 432, clientY: 114 });
    // The far left, the top: 20 Hz, +12 dB
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: 10, clientY: 0 });
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: 38, clientY: 10 });
    // Shown while dragged, not saved yet
    await within(wh).findByRole("button", { name: /^Point 1: Peak, 20 Hz, \+12.0 dB/ });
    expect(saved().length).toBe(before);
    fireEvent.pointerUp(svg, { pointerId: 1 });
    await waitFor(() => expect(lastEq().bands[0]).toMatchObject({ frequency: 20, gain: 12 }));
    expect(saved().length).toBe(before + 1);
  });

  it("turns off and keeps its points", async () => {
    const { wh, lastEq } = await eqOn(true);
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    await waitFor(() => expect(lastEq().bands).toHaveLength(1));
    fireEvent.click(within(wh).getByRole("switch", { name: t("eq.enable") }));
    await waitFor(() => expect(lastEq().enabled).toBe(false));
    expect(lastEq().bands).toHaveLength(1);
    // Still there, greyed out
    await waitFor(() => expect(wh.querySelector(".eq-graph.disabled")).toBeTruthy());
    expect(within(wh).getByRole("button", { name: /^Point 1:/ })).toHaveAttribute("aria-disabled", "true");
    for (const name of [t("eq.add_point"), t("eq.reset"), t("eq.export")]) {
      expect(within(wh).getByRole("button", { name })).toBeDisabled();
    }
    expect(within(wh).getByRole("combobox", { name: t("eq.point") })).toBeDisabled();
  });

  it("keeps its fields empty until a point is picked, from the graph or the Point box", async () => {
    const { wh, lastEq, synced } = await eqOn(true);
    const point = () => within(wh).getByRole("combobox", { name: t("eq.point") }) as HTMLSelectElement;
    const freq = () => within(wh).getByRole("spinbutton", { name: t("eq.frequency") }) as HTMLInputElement;
    // No points: everything below the graph is there, disabled and empty
    expect(point()).toBeDisabled();
    expect(within(wh).getByRole("combobox", { name: t("eq.type") })).toBeDisabled();
    expect(freq()).toBeDisabled();
    expect(freq().value).toBe("");
    expect(within(wh).getByRole("button", { name: t("eq.delete_point") })).toBeDisabled();
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    await waitFor(() => expect(lastEq().bands).toHaveLength(1));
    await synced();
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    await waitFor(() => expect(lastEq().bands).toHaveLength(2));
    await synced();
    // The new point is selected; a click on the graph's background lets go
    await waitFor(() => expect(point().value).toBe("1"));
    expect(freq()).toHaveValue(2000);
    fireEvent.pointerDown(wh.querySelector(".eq-graph svg")!, { button: 0 });
    await waitFor(() => expect(freq()).toBeDisabled());
    expect(freq().value).toBe("");
    expect(point().selectedOptions[0]?.textContent).toBe("");
    // The Point box picks one
    expect(point()).toBeEnabled();
    fireEvent.change(point(), { target: { value: "0" } });
    await waitFor(() => expect(freq()).toHaveValue(1000));
    expect(within(wh).getByRole("button", { name: /^Point 1:/ })).toHaveAttribute("aria-pressed", "true");
    expect(within(wh).getByRole("combobox", { name: t("eq.type") })).toHaveValue("Peaking");
  });

  it("exports its points for Equalizer APO", async () => {
    const { wh, lastEq } = await eqOn(true);
    const exportButton = within(wh).getByRole("button", { name: t("eq.export") });
    expect(exportButton).toBeDisabled();
    fireEvent.click(within(wh).getByRole("button", { name: t("eq.add_point") }));
    await waitFor(() => expect(lastEq().bands).toHaveLength(1));
    await waitFor(() => expect(exportButton).toBeEnabled());
    fireEvent.click(exportButton);
    await waitFor(() => expect(mock!.savedFiles).toHaveLength(1));
    expect(mock!.savedFiles[0].name).toBe("BlueCaster EQ.txt");
    expect(mock!.savedFiles[0].text.split(String.fromCharCode(13, 10))).toEqual([
      "Preamp: 0.0 dB",
      "Filter 1: ON PK Fc 1000 Hz Gain 0.0 dB Q 1.00",
      "",
    ]);
  });
});
