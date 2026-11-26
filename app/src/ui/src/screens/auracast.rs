//! Auracast (Developer screen): broadcasts the test tone, the system audio,
//! an output device's sound or a recording device to every receiver nearby
//! that joins (stream.start_broadcast; the core refuses it while developer
//! mode is off), at a volume of its own (the stream's, which can follow the
//! Windows volume). The broadcast is a stream like any other: Stop and the
//! status follow the core's stream, the choices outlive the view (one set
//! per core connection).

use std::time::Duration;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Entity, EntityId, Global, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use ipc::{AudioDevice, AuracastReceiverCandidate, AuracastReceiverStatus, AuracastReceivers, Method, StreamStatus};
use ui_model::i18n::t;
use ui_model::t;
use ui_model::view::auracast::{
    Arrangement, Channels, Choices, DeviceNames, MAX_BIGS, PairMode, Quality, ReceiverPick, SOURCE_SETTLE_MS, Session,
    SingleLayout, Source, SourceParams, arrangements, auracast_adapter_choices, followable, receiver_options,
    receiver_status_text, settled_source, show_adapter, source_options, source_params,
};

use crate::app;
use crate::core::Core;
use crate::screens::auracast_mixer::AuracastMixer;
use crate::theme::{self, ActivePalette as _};
use crate::ui::stream::VolumeControl;
use crate::ui::{self, ButtonKind, Choice, IconName, icon};

/// The choices and the broadcast asked for here, one set per core
/// connection: they follow the stream while no view shows them
pub struct AuracastSession {
    choices: Choices,
    s: Session,
    /// The names of the devices listed so far, for a chosen one that is gone
    names: DeviceNames,
    _follow: Subscription,
}

struct Sessions {
    core: EntityId,
    session: Entity<AuracastSession>,
}

impl Global for Sessions {}

fn stream_of(core: &Entity<Core>, cx: &App) -> StreamStatus {
    core.read(cx).snap().map(|s| s.stream.clone()).unwrap_or_default()
}

fn session(core: &Entity<Core>, cx: &mut App) -> Entity<AuracastSession> {
    if let Some(s) = cx.try_global::<Sessions>().filter(|s| s.core == core.entity_id()) {
        return s.session.clone();
    }
    let session = cx.new(|cx| {
        let follow = cx.observe(core, |this: &mut AuracastSession, core, cx| {
            let st = stream_of(&core, cx);
            let before = this.s.clone();
            this.s.follow(&st);
            if this.s != before {
                cx.notify();
            }
        });
        AuracastSession {
            choices: Choices::default(),
            s: Session::default(),
            names: DeviceNames::new(),
            _follow: follow,
        }
    });
    cx.set_global(Sessions { core: core.entity_id(), session: session.clone() });
    session
}

pub struct Auracast {
    core: Entity<Core>,
    session: Entity<AuracastSession>,
    name: Entity<InputState>,
    code: Entity<InputState>,
    mixer: Entity<AuracastMixer>,
    /// The broadcast's volume (the stream's), off while none is live
    volume: Entity<VolumeControl>,
    /// Windows' devices to broadcast from, read again when they change
    outputs: Vec<AudioDevice>,
    inputs: Vec<AudioDevice>,
    audio_key: String,
    settle: Option<Task<()>>,
    /// The saved LE Audio devices to choose receivers from, and what the
    /// broadcast's receivers do (auracast.receivers, read every second)
    candidates: Vec<AuracastReceiverCandidate>,
    live_receivers: Vec<AuracastReceiverStatus>,
    _poll: Task<()>,
    _subs: Vec<Subscription>,
}

/// How often the receivers are read again
const RECEIVERS_POLL: Duration = Duration::from_secs(1);

impl Auracast {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Auracast {
        let core = app::core(cx);
        let session = session(&core, cx);
        let c = session.read(cx).choices.clone();
        let name = cx.new(|cx| InputState::new(window, cx).default_value(c.name.clone()));
        let code = cx.new(|cx| {
            InputState::new(window, cx).default_value(c.code.clone()).placeholder(t("ui.dev.auracast_code_none"))
        });
        let mixer = cx.new(|cx| AuracastMixer::new(window, cx));
        let volume = cx.new(|cx| VolumeControl::new(true, window, cx));
        let audio_key = ui::audio_devices_key(core.read(cx).snap());
        let subs = vec![
            cx.observe(&core, |this: &mut Self, core, cx| {
                let key = ui::audio_devices_key(core.read(cx).snap());
                if key != this.audio_key {
                    this.audio_key = key;
                    this.read_devices(cx);
                }
            }),
            cx.observe_in(&core, window, |this, _, window, cx| this.sync_mixer(window, cx)),
            cx.observe_in(&session, window, |this, _, window, cx| this.sync_mixer(window, cx)),
            cx.observe(&mixer, |_, _, cx| cx.notify()),
            cx.subscribe(&name, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = input.read(cx).value().to_string();
                    this.set(cx, move |c| c.name = v);
                }
            }),
            cx.subscribe(&code, |this, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = input.read(cx).value().to_string();
                    this.set(cx, move |c| c.code = v);
                }
            }),
        ];
        let poll = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                let Ok(task) =
                    this.update(cx, |_, cx| app::request::<AuracastReceivers>(Method::AuracastReceivers, cx))
                else {
                    return;
                };
                if let Ok(r) = task.await {
                    let gone = this
                        .update(cx, |this, cx| {
                            if this.candidates != r.candidates || this.live_receivers != r.live {
                                this.candidates = r.candidates;
                                this.live_receivers = r.live;
                                cx.notify();
                            }
                        })
                        .is_err();
                    if gone {
                        return;
                    }
                }
                cx.background_executor().timer(RECEIVERS_POLL).await;
            }
        });
        let mut this = Auracast {
            core,
            session,
            name,
            code,
            mixer,
            volume,
            outputs: Vec::new(),
            inputs: Vec::new(),
            audio_key,
            settle: None,
            candidates: Vec::new(),
            live_receivers: Vec::new(),
            _poll: poll,
            _subs: subs,
        };
        this.sync_mixer(window, cx);
        this.read_devices(cx);
        this.read_follow(cx);
        this
    }

    fn stream(&self, cx: &App) -> StreamStatus {
        stream_of(&self.core, cx)
    }

    /// Windows' output and recording devices; their names are kept for a
    /// chosen one that disappears
    fn read_devices(&mut self, cx: &mut Context<Self>) {
        let outputs = app::request::<Vec<AudioDevice>>(Method::AudioDevices, cx);
        let inputs = app::request::<Vec<AudioDevice>>(Method::AudioInputDevices, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let outputs = outputs.await.ok();
            let inputs = inputs.await.ok();
            let _ = this.update(cx, |this, cx| {
                this.session.update(cx, |s, _| {
                    for d in outputs.iter().chain(inputs.iter()).flatten() {
                        s.names.insert(d.id.clone(), d.name.clone());
                    }
                });
                if let Some(list) = outputs {
                    this.outputs = list;
                }
                if let Some(list) = inputs {
                    this.inputs = list;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The core keeps "follow" between broadcasts and windows: the switch shows it
    fn read_follow(&mut self, cx: &mut Context<Self>) {
        let task = app::request::<bool>(Method::AuracastFollowVolume, cx);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            if let Ok(follow) = task.await {
                let _ = this.update(cx, |this, cx| this.set(cx, |c| c.follow = follow));
            }
        })
        .detach();
    }

    /// The source as the core takes it
    fn params_of(&self, source: &Source, cx: &App) -> SourceParams {
        source_params(source, &self.outputs, &self.inputs, &self.session.read(cx).names)
    }

    fn set(&mut self, cx: &mut Context<Self>, patch: impl FnOnce(&mut Choices)) {
        self.session.update(cx, |s, cx| {
            patch(&mut s.choices);
            cx.notify();
        });
    }

    /// The mixer shows with a matrix; it reads its state again when the rear
    /// or the broadcast's state changes
    fn sync_mixer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let st = self.stream(cx);
        let s = self.session.read(cx);
        let live = s.s.live(&st);
        let rear = !s.choices.matrix() || s.choices.rear != PairMode::Off;
        let props = s.choices.mixed().then_some((rear, live && !s.s.asking));
        self.mixer.update(cx, |m, cx| m.set_props(props, window, cx));
        self.volume.update(cx, |v, cx| v.set_off(!live, cx));
        cx.notify();
    }

    fn start_stop(&mut self, cx: &mut Context<Self>) {
        let st = self.stream(cx);
        let session = self.session.clone();
        if session.read(cx).s.live(&st) {
            let task = app::run(app::request::<serde_json::Value>(Method::StreamStop, cx), cx);
            cx.spawn(async move |_, cx: &mut AsyncApp| {
                task.await;
                session.update(cx, |s, cx| {
                    s.s.mine = false;
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let sent = self.params_of(&session.read(cx).choices.source, cx);
        let request = session.update(cx, |s, cx| {
            s.s.asking = true;
            s.s.failed = None;
            s.s.on_air = false;
            s.s.mine = true;
            cx.notify();
            s.choices.start_request(sent)
        });
        let task = app::run(app::request::<serde_json::Value>(request, cx), cx);
        let core = self.core.clone();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let ok = task.await.is_some();
            session.update(cx, |s, cx| {
                if !ok {
                    s.s.mine = false;
                }
                s.s.asking = false;
                let st = stream_of(&core, cx);
                s.s.follow(&st);
                cx.notify();
            });
        })
        .detach();
    }

    /// The source: the running broadcast switches to it in place
    fn change_source(&mut self, source: Source, cx: &mut Context<Self>) {
        let before = self.session.read(cx).choices.source.clone();
        let chosen = source.clone();
        self.set(cx, |c| c.source = chosen);
        let st = self.stream(cx);
        if !self.session.read(cx).s.live(&st) {
            return;
        }
        let sent = self.params_of(&source, cx);
        let sent_before = self.params_of(&before, cx);
        let task = app::run(app::request::<bool>(sent.set_source_request(), cx), cx);
        self.settle = Some(cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let taken = task.await.unwrap_or(false);
            if !taken {
                // Not taken (a start or stop under way): the broadcast keeps its source
                let _ = this.update(cx, |this, cx| {
                    if this.session.read(cx).choices.source == source {
                        this.set(cx, |c| c.source = before);
                    }
                });
                return;
            }
            // A source that cannot be captured: the broadcast goes on with
            // the one before, which its stream info then still names
            cx.background_executor().timer(Duration::from_millis(SOURCE_SETTLE_MS)).await;
            let _ = this.update(cx, |this, cx| {
                let st = this.stream(cx);
                let s = this.session.read(cx);
                let info = st.info.as_ref().and_then(|i| i.source.as_ref());
                let back = settled_source(
                    s.s.live(&st),
                    &s.choices.source,
                    (&source, &sent),
                    (&before, &sent_before),
                    info,
                    &this.outputs,
                    &this.inputs,
                );
                if let Some(back) = back {
                    this.set(cx, |c| c.source = back);
                }
            });
        }));
    }

    /// Follow the Windows volume: kept for the next broadcast, taken by the running one
    fn change_follow(&mut self, follow: bool, cx: &mut Context<Self>) {
        self.set(cx, |c| c.follow = follow);
        app::send(Method::AuracastSetFollowVolume { follow }, cx);
    }
}

/// A line with its label and a few choices side by side
fn choice<T: Clone + PartialEq + 'static>(
    id: &'static str,
    label: String,
    value: &T,
    disabled: bool,
    options: Vec<(T, String)>,
    on_change: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    ui::row().child(div().child(label)).child(ui::segmented(
        id,
        value,
        options.into_iter().map(|(v, l)| Choice::new(v, l).disabled(disabled)).collect(),
        on_change,
    ))
}

/// A pair of matrix outputs: how it is sent, and its rate
#[allow(clippy::too_many_arguments)]
fn pair_row(
    id: &'static str,
    label: String,
    mode: PairMode,
    rate: Quality,
    disabled: bool,
    on_mode: impl Fn(&PairMode, &mut Window, &mut App) + 'static,
    on_rate: impl Fn(&Quality, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let modes = [
        (PairMode::Stereo, "ui.dev.auracast_pair_stereo"),
        (PairMode::Separate, "ui.dev.auracast_pair_separate"),
        (PairMode::Subgroups, "ui.dev.auracast_pair_subgroups"),
        (PairMode::Off, "ui.dev.auracast_pair_off"),
    ];
    let rates =
        [(Quality::Hq, "ui.dev.auracast_48"), (Quality::Mq, "ui.dev.auracast_32"), (Quality::Sq, "ui.dev.auracast_24")];
    ui::row().child(div().child(label)).child(
        div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap(theme::S2)
            .child(ui::segmented(
                SharedString::from(format!("{id}-mode")),
                &mode,
                modes.into_iter().map(|(v, k)| Choice::new(v, t(k)).disabled(disabled)).collect(),
                on_mode,
            ))
            .child(ui::segmented(
                SharedString::from(format!("{id}-rate")),
                &rate,
                rates
                    .into_iter()
                    .map(|(v, k)| Choice::new(v, t(k)).disabled(disabled || mode == PairMode::Off))
                    .collect(),
                on_rate,
            )),
    )
}

impl Render for Auracast {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *cx.palette();
        let st = self.stream(cx);
        let Some(snap) = self.core.read(cx).snap().cloned() else { return div().into_any_element() };
        let (c, s) = {
            let x = self.session.read(cx);
            (x.choices.clone(), x.s.clone())
        };
        let live = s.live(&st);
        let locked = s.locked(&st);
        let (name_error, code_error) = (c.name_error(), c.code_error());
        let session = self.session.downgrade();
        let set = move |patch: Box<dyn FnOnce(&mut Choices)>, cx: &mut App| {
            let _ = session.update(cx, |s, cx| {
                patch(&mut s.choices);
                cx.notify();
            });
        };
        let set = std::rc::Rc::new(set);

        let start = ui::button(
            "auracast-start",
            if live { t("receiver.stop") } else { t("receiver.start") },
            if live { ButtonKind::Danger } else { ButtonKind::Strong },
        )
        .icon(icon(if live { IconName::Stop } else { IconName::Play }))
        .loading(s.asking)
        .disabled(c.start_disabled(live))
        .on_click(cx.listener(|this, _, _, cx| this.start_stop(cx)));

        // The adapter the broadcast goes on: offered while several adapters
        // are on WinUSB, or while one is chosen (to go back to the current one)
        let chosen = snap.settings.auracast_adapter_id.clone();
        let adapters = &snap.adapter.adapters;
        let adapter = show_adapter(adapters, &chosen).then(|| {
            let (value, options) = auracast_adapter_choices(adapters, &chosen);
            ui::field(
                t("ui.dev.auracast_adapter"),
                Some(t("ui.dev.auracast_adapter_hint").into()),
                None,
                ui::select(
                    "auracast-adapter",
                    &value,
                    options.into_iter().map(|(v, l)| Choice::new(v, l)).collect(),
                    locked,
                    |v: &String, _, cx| {
                        let v = v.clone();
                        app::save_settings(move |s| s.auracast_adapter_id = v, cx);
                    },
                ),
                cx,
            )
        });

        let status = s.status(&st).map(|(text, bad)| {
            let summary =
                live.then(|| st.info.as_ref().and_then(|i| i.le.as_ref()).map(|l| l.summary.clone())).flatten();
            div()
                .flex()
                .flex_col()
                .gap(theme::S1)
                .when(bad, |d| d.text_color(p.bad_text))
                .child(text)
                .when_some(summary.filter(|x| !x.is_empty()), |d, sum| d.child(ui::muted(sum, cx)))
        });

        let set_q = set.clone();
        let set_ch = set.clone();
        let sources = {
            let names = &self.session.read(cx).names;
            source_options(&c.source, &self.outputs, &self.inputs, names)
                .into_iter()
                .map(|o| {
                    let choice = Choice::new(o.value, o.label);
                    match o.group {
                        Some(g) => choice.group(g),
                        None => choice,
                    }
                })
                .collect()
        };
        let this = cx.entity().downgrade();
        let this_follow = this.clone();
        let form = ui::section(t("ui.dev.auracast"), Some(start.into_any_element()), cx)
            .children(adapter)
            .child(ui::field(
                t("ui.dev.auracast_name"),
                None,
                name_error.map(Into::into),
                Input::new(&self.name).aria_label(t("ui.dev.auracast_name")).disabled(locked).w(px(260.)),
                cx,
            ))
            .child(ui::field(
                t("ui.dev.auracast_code"),
                None,
                code_error.map(Into::into),
                Input::new(&self.code).aria_label(t("ui.dev.auracast_code")).disabled(locked).w(px(260.)),
                cx,
            ))
            .child(choice(
                "auracast-quality",
                t("ui.dev.auracast_quality"),
                &c.quality,
                locked || c.matrix(),
                vec![
                    (Quality::Hq, t("ui.dev.auracast_hq")),
                    (Quality::Mq, t("ui.dev.auracast_mq")),
                    (Quality::Sq, t("ui.dev.auracast_sq")),
                ],
                move |v: &Quality, _, cx| {
                    let v = *v;
                    set_q(Box::new(move |c| c.quality = v), cx);
                },
            ))
            .child(choice(
                "auracast-channels",
                t("ui.dev.auracast_channels"),
                &c.channels,
                locked,
                vec![
                    (Channels::Stereo, t("ui.dev.auracast_stereo")),
                    (Channels::Mono, t("ui.dev.auracast_mono")),
                    (Channels::Split, t("ui.dev.auracast_split")),
                    (Channels::Matrix, t("ui.dev.auracast_matrix")),
                    (Channels::Single, t("ui.dev.auracast_single")),
                ],
                move |v: &Channels, _, cx| {
                    let v = *v;
                    set_ch(Box::new(move |c| c.set_channels(v)), cx);
                },
            ))
            .when(c.channels == Channels::Single, |d| {
                let set_single = set.clone();
                d.child(ui::field(
                    t("ui.dev.auracast_single_layout"),
                    Some(t("ui.dev.auracast_single_layout_hint").into()),
                    None,
                    ui::select(
                        "auracast-single",
                        &c.single,
                        SingleLayout::ALL.into_iter().map(|v| Choice::new(v, t(v.label_key()))).collect(),
                        locked,
                        move |v: &SingleLayout, _, cx| {
                            let v = *v;
                            set_single(Box::new(move |c| c.single = v), cx);
                        },
                    ),
                    cx,
                ))
            })
            .child(ui::field(
                t("receiver.source"),
                None,
                None,
                ui::select("auracast-source", &c.source, sources, s.source_locked(&st), move |v: &Source, _, cx| {
                    let v = v.clone();
                    let _ = this.update(cx, |this, cx| this.change_source(v, cx));
                }),
                cx,
            ))
            .child(
                ui::row()
                    .child(div().child(t("ui.dev.auracast_volume")))
                    .child(div().w(px(260.)).child(self.volume.clone())),
            )
            .child(ui::toggle(
                "auracast-follow",
                t("ui.dev.auracast_follow"),
                Some(t("ui.dev.auracast_follow_hint").into()),
                c.follow && followable(&c.source),
                !followable(&c.source),
                move |v: &bool, _, cx| {
                    let v = *v;
                    let _ = this_follow.update(cx, |this, cx| this.change_follow(v, cx));
                },
                cx,
            ))
            .children(status);

        let receivers = self.receivers_section(&c, locked, &set, cx);

        let mixer = c.mixed().then(|| {
            let bigs = c.bigs();
            let (set_fm, set_fr, set_rm, set_rr) = (set.clone(), set.clone(), set.clone(), set.clone());
            let pairs: Vec<AnyElement> = vec![
                pair_row(
                    "auracast-front",
                    t("ui.dev.auracast_front"),
                    c.front,
                    c.front_quality,
                    locked,
                    move |v, _, cx| {
                        let v = *v;
                        set_fm(Box::new(move |c| c.front = v), cx);
                    },
                    move |v, _, cx| {
                        let v = *v;
                        set_fr(Box::new(move |c| c.front_quality = v), cx);
                    },
                )
                .into_any_element(),
                pair_row(
                    "auracast-rear",
                    t("ui.dev.auracast_rear"),
                    c.rear,
                    c.rear_quality,
                    locked,
                    move |v, _, cx| {
                        let v = *v;
                        set_rm(Box::new(move |c| c.rear = v), cx);
                    },
                    move |v, _, cx| {
                        let v = *v;
                        set_rr(Box::new(move |c| c.rear_quality = v), cx);
                    },
                )
                .into_any_element(),
            ];
            let mut children = if c.matrix() { pairs } else { Vec::new() };
            children.push(ui::hint(t("ui.dev.auracast_speakers_hint"), cx).into_any_element());
            if c.matrix() && bigs == 0 {
                children
                    .push(ui::hint(t("ui.dev.auracast_pairs_off_hint"), cx).text_color(p.bad_text).into_any_element());
            }
            if bigs > MAX_BIGS {
                children.push(
                    div()
                        .flex()
                        .items_center()
                        .gap(theme::S1)
                        .text_size(theme::FS_SMALL)
                        .text_color(p.warn_text)
                        .child(t!("ui.dev.auracast_bigs_hint", bigs, MAX_BIGS))
                        .into_any_element(),
                );
            }
            self.mixer.update(cx, |m, cx| m.section(children, cx))
        });

        div()
            .flex()
            .flex_col()
            .gap(theme::S4)
            .child(ui::hint(t("ui.dev.auracast_intro"), cx))
            .child(form)
            .child(receivers)
            .children(mixer.flatten())
            .into_any_element()
    }
}

impl Auracast {
    /// The receivers an Auracast assistant tells: who plays what, a saved
    /// device for each place, and what each does while the broadcast runs
    fn receivers_section(
        &self,
        c: &Choices,
        locked: bool,
        set: &std::rc::Rc<impl Fn(Box<dyn FnOnce(&mut Choices)>, &mut App) + 'static>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = *cx.palette();
        let offered = arrangements(c.channels);
        let mut section = ui::section(t("ui.dev.auracast_receivers"), None, cx)
            .child(ui::hint(t("ui.dev.auracast_receivers_hint"), cx));
        if offered.len() > 1 {
            let set_a = set.clone();
            section = section.child(choice(
                "auracast-arrangement",
                t("ui.dev.auracast_arrangement"),
                &c.arrangement(),
                locked,
                offered.iter().map(|a| (*a, t(a.label_key()))).collect(),
                move |v: &Arrangement, _, cx| {
                    let v = *v;
                    set_a(Box::new(move |c| c.arrangement = v), cx);
                },
            ));
        }
        let options = receiver_options(&self.candidates);
        for slot in c.slots() {
            let chosen = c.receiver_of(&slot).map(|r| r.address.clone()).unwrap_or_default();
            let mut choices: Vec<Choice<String>> =
                options.iter().map(|(v, l)| Choice::new(v.clone(), l.clone())).collect();
            // One chosen before that is no longer listed (removed, no BASS)
            if !chosen.is_empty() && !options.iter().any(|(v, _)| *v == chosen) {
                let name = c.receiver_of(&slot).map(|r| r.name.clone()).unwrap_or_default();
                choices.push(Choice::new(chosen.clone(), if name.is_empty() { chosen.clone() } else { name }));
            }
            let set_r = set.clone();
            let names: Vec<(String, String)> =
                self.candidates.iter().map(|c| (c.address.clone(), c.name.clone())).collect();
            let key = slot.key;
            section = section.child(ui::field(
                t(slot.label_key),
                None,
                None,
                ui::select(
                    SharedString::from(format!("auracast-rx-{key}")),
                    &chosen,
                    choices,
                    locked,
                    move |v: &String, _, cx| {
                        let pick = (!v.is_empty()).then(|| ReceiverPick {
                            address: v.clone(),
                            name: names.iter().find(|(a, _)| a == v).map(|(_, n)| n.clone()).unwrap_or_default(),
                        });
                        set_r(Box::new(move |c| c.set_receiver(key, pick)), cx);
                    },
                ),
                cx,
            ));
        }
        if let Some(e) = c.receiver_error() {
            section = section.child(ui::hint(e, cx).text_color(p.bad_text));
        }
        if !self.live_receivers.is_empty() {
            let lines = self.live_receivers.iter().map(|r| {
                let bad = matches!(r.phase.as_str(), "failed" | "not_found" | "no_bass");
                div().when(bad, |d| d.text_color(p.bad_text)).child(receiver_status_text(r))
            });
            section = section.child(div().flex().flex_col().gap(theme::S1).text_size(theme::FS_SMALL).children(lines));
        }
        section.into_any_element()
    }
}

#[cfg(test)]
mod interaction_tests;
