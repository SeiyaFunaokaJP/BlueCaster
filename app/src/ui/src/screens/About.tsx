// About: the program, its version, licence and where to find the author;
// then, in a box of its own (issue #86), the third-party software: the
// libraries' licences and the dialog of VB-Audio's cables, which names them
// as VB-Audio's donationware.

import { createSignal } from "solid-js";
import { useApp } from "../app/context";
import { CablesDialog } from "../components/CableNotes";
import { Button, Section } from "../components/controls";
import { t } from "../i18n";
import type { AudioDevice } from "../ipc/types";
import { LINKS, thirdPartyUrl } from "../lib/links";

export function About() {
  const app = useApp();
  const { core } = app;
  const update = () => core.state.snap!.update;
  // The cables' dialog, with Windows' outputs read when it opens
  const [cables, setCables] = createSignal(false);
  const [devices, setDevices] = createSignal<AudioDevice[] | null>(null);
  const loadDevices = () =>
    core
      .request("audio.devices")
      .then(setDevices)
      .catch(() => {});
  // The licence texts shipped next to the programs; a development build
  // has none, so the release's copy online
  const openLicenses = () => {
    const dir = core.transport.licensesDir;
    void (dir ? core.transport.openFolder(dir) : core.transport.openUrl(thirdPartyUrl(core.transport.version)));
  };
  return (
    <div class="page">
      <header class="page-head">
        <h1>{t("ui.nav.about")}</h1>
      </header>
      <Section title={t("help.about_title")}>
        <div class="about">
          <img src="/app-icon.png" alt="" width="48" height="48" />
          <div>
            <strong>{t("help.full_name")}</strong>
            <p class="muted num">
              {t("help.version")} {core.transport.version} · {t("ui.settings.core_version", update().current_version)}
            </p>
            <p class="muted">{t("help.author")}</p>
          </div>
        </div>
        <p class="hint">{t("help.license")}</p>
        <p class="hint">{t("ui.settings.icons_license")}</p>
        <div class="inline wrap">
          <Button icon="open" kind="subtle" onClick={() => core.transport.openUrl(LINKS.repo)}>
            {t("help.github")}
          </Button>
          <Button icon="open" kind="subtle" onClick={() => core.transport.openUrl(LINKS.issues)}>
            {t("menu.help.report_issue")}
          </Button>
          <Button icon="open" kind="subtle" onClick={() => core.transport.openUrl(LINKS.sponsor)}>
            {t("help.sponsor")}
          </Button>
        </div>
      </Section>
      <Section title={t("help.third_party_title")}>
        <p class="hint">{t("help.binary_license")}</p>
        <p class="hint">{t("help.third_party")}</p>
        <div class="inline wrap">
          <Button icon="open" kind="subtle" onClick={() => openLicenses()}>
            {t("help.third_party_link")}
          </Button>
          <Button
            kind="subtle"
            onClick={() => {
              setDevices(null);
              void loadDevices();
              setCables(true);
            }}
          >
            {t("cables.title")}
          </Button>
        </div>
      </Section>
      <CablesDialog
        open={cables()}
        devices={devices()}
        reload={() => void loadDevices()}
        onClose={() => setCables(false)}
      />
    </div>
  );
}
