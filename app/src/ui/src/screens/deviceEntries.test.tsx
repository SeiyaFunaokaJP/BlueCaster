// Audio Devices: each device's settings are its own (issue #86: the
// settings of one device showed on another, and were saved into it, once
// the core's list changed order; the store reconciles it by position)

import { cleanup, render, waitFor, within } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "../app/App";
import { setLanguage, t } from "../i18n";
import { MockCore } from "../ipc/mock";
import type { Device, Event } from "../ipc/types";
import { createCore } from "../ipc/store";
import { endSettle } from "./Codecs";

let mock: MockCore | null = null;

afterEach(async () => {
  cleanup();
  endSettle();
  mock?.dispose();
  mock = null;
  await setLanguage("en");
});

const entries = () => [...document.querySelectorAll<HTMLElement>("main .device-entry")];
const entry = (name: RegExp) => entries().find((e) => name.test(e.textContent ?? ""))!;
const source = (e: HTMLElement) => (within(e).getByLabelText(t("capture.mode")) as HTMLSelectElement).value;

describe("Audio Devices entries", () => {
  it("keep each device's own source when the core's list changes order", async () => {
    location.hash = "#/devices";
    mock = new MockCore({ scenario: "capped", speed: 0 });
    const core = createCore(mock);
    render(() => <App core={core} />);
    await waitFor(() => expect(entries().length).toBe(5));
    // SRS-XB13 captures Spotify; moto buds+ has no profile yet
    await waitFor(() => expect(source(entry(/SRS-XB13/))).toBe("app:spotify.exe"));
    expect(source(entry(/moto buds\+/))).toBe("loopback");
    // The core sends a list in another order (a device forgotten or added
    // before them), as new objects
    const list = JSON.parse(JSON.stringify(core.state.snap!.devices)) as Device[];
    [list[1], list[2]] = [list[2], list[1]];
    (mock as unknown as { emit: (e: Event) => void }).emit({ kind: "devices", value: list });
    await waitFor(() => expect(entries()[1].textContent).toMatch(/moto buds\+/));
    expect(source(entry(/moto buds\+/))).toBe("loopback");
    expect(source(entry(/SRS-XB13/))).toBe("app:spotify.exe");
  });
});
