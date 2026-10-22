//! Mission selection and a form editor with an editable TOML view.

// Layout limits bound the numeric casts below.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
use crate::{client::EngineClient, widgets};
use openatc_core::{
    missions::{Media, Mission},
    state::State,
};
use serde_json::json;

#[derive(Default)]
pub struct Browser {
    catalogue: Vec<Mission>,
    loaded: bool,
    filter: String,
    compatible_only: bool,
    selected: Option<Mission>,
    source: String,
    advanced: bool,
    notice: String,
}

impl Browser {
    pub fn page(&mut self, ui: &imgui::Ui, engine: &EngineClient, state: &State) {
        if !self.loaded {
            self.loaded = true;
            self.compatible_only = true;
            engine.post("/missions/list", json!({}));
        }
        if let Some(reply) = engine.take_reply("/missions/list") {
            if reply.success {
                self.catalogue =
                    serde_json::from_value(reply.data["missions"].clone()).unwrap_or_default();
                self.notice = reply.data["errors"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| e.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
            } else {
                self.notice = reply.error;
            }
        }
        for path in [
            "/missions/save",
            "/missions/start",
            "/missions/stop",
            "/missions/choose",
        ] {
            if let Some(reply) = engine.take_reply(path) {
                self.notice = if reply.success {
                    "Mission updated.".into()
                } else {
                    reply.error
                };
                if path == "/missions/save" && reply.success {
                    engine.post("/missions/list", json!({}));
                }
            }
        }
        if !self.notice.is_empty() {
            ui.text_wrapped(&self.notice);
        }
        if state.mission.running {
            widgets::section_title(ui, &state.mission.title);
            ui.text_wrapped("The story follows your flight. Requests to ATC and aircraft controls still use the normal communication controls.");
            for choice in &state.mission.choices {
                let _disabled = ui.begin_disabled(state.telemetry.radio_busy);
                if ui.button_with_size(&choice.label, [ui.content_region_avail()[0], 0.0]) {
                    engine.post("/missions/choose", json!({"choice":choice.id}));
                }
            }
            if ui.small_button("End mission") {
                engine.post("/missions/stop", json!({}));
            }
        }
        if let Some(_tabs) = ui.tab_bar("MissionViews") {
            if let Some(_tab) = ui.tab_item("Choose a flight") {
                self.catalogue(ui, engine, state);
            }
            if let Some(_tab) = ui.tab_item("Mission editor") {
                self.editor(ui, engine);
            }
            if let Some(_tab) = ui.tab_item("Flight journal") {
                if state.mission.debrief.is_empty() {
                    ui.text_disabled("Mission events and your responses appear here.");
                }
                for event in &state.mission.debrief {
                    ui.text_wrapped(event);
                    ui.separator();
                }
            }
        }
    }

    fn catalogue(&mut self, ui: &imgui::Ui, engine: &EngineClient, state: &State) {
        ui.input_text(field_label(ui, "Search missions"), &mut self.filter)
            .build();
        ui.checkbox("Suitable for my aircraft", &mut self.compatible_only);
        ui.same_line();
        if ui.small_button("Refresh") {
            engine.post("/missions/list", json!({}));
        }
        let aircraft = if state.telemetry.aircraft_icao.is_empty() {
            &state.plan.aircraft
        } else {
            &state.telemetry.aircraft_icao
        };
        widgets::muted_wrapped(
            ui,
            format!("Aircraft: {aircraft} / routes are suggestions, not filed flight plans"),
        );
        for mission in &self.catalogue {
            if self.compatible_only && !mission.compatible(aircraft) {
                continue;
            }
            let search = format!(
                "{} {} {} {}",
                mission.title,
                mission.departure,
                mission.destination,
                mission.aircraft.join(" ")
            )
            .to_lowercase();
            if !search.contains(&self.filter.to_lowercase()) {
                continue;
            }
            let _id = ui.push_id(&mission.id);
            ui.separator();
            ui.text_colored(widgets::accent(ui), &mission.title);
            ui.text_wrapped(format!(
                "{} → {} · about {} min · {}",
                mission.departure,
                mission.destination,
                mission.duration_minutes,
                mission.aircraft.join(", ")
            ));
            ui.text_wrapped(&mission.briefing);
            {
                let _disabled = ui.begin_disabled(
                    state.mission.running
                        || !mission.compatible(aircraft)
                        || !matches!(
                            state.phase,
                            openatc_core::state::PhaseCode::Parked
                                | openatc_core::state::PhaseCode::Clearance
                        ),
                );
                if ui.button("Start mission") {
                    engine.post("/missions/start", json!({"id":mission.id}));
                }
            }
            ui.same_line();
            if ui.button("Edit a copy") {
                let mut copy = mission.clone();
                copy.id = format!("{}-custom", copy.id);
                copy.title.push_str(" (custom)");
                self.source = toml::to_string_pretty(&copy).unwrap_or_default();
                self.selected = Some(copy);
                self.advanced = false;
                self.notice = "Copy ready in Mission editor. Save it to keep your changes.".into();
            }
        }
    }

    fn editor(&mut self, ui: &imgui::Ui, engine: &EngineClient) {
        let Some(mission) = &mut self.selected else {
            ui.text_wrapped("Choose Edit a copy beside a mission to create your own flight story.");
            return;
        };
        let was_advanced = self.advanced;
        ui.checkbox("Advanced TOML source", &mut self.advanced);
        if self.advanced != was_advanced {
            if self.advanced {
                self.source = toml::to_string_pretty(mission).unwrap_or_default();
            } else {
                match openatc_core::missions::parse(&self.source) {
                    Ok(parsed) => *mission = parsed,
                    Err(e) => {
                        self.notice = e;
                        self.advanced = true;
                    }
                }
            }
        }
        if self.advanced {
            ui.input_text_multiline(
                "##MissionSource",
                &mut self.source,
                [ui.content_region_avail()[0], 320.0],
            )
            .build();
        } else {
            ui.input_text(field_label(ui, "File ID"), &mut mission.id)
                .build();
            ui.input_text(field_label(ui, "Title"), &mut mission.title)
                .build();
            ui.input_text_multiline(
                field_label(ui, "Briefing"),
                &mut mission.briefing,
                [ui.content_region_avail()[0], 100.0],
            )
            .build();
            let mut aircraft = mission.aircraft.join(", ");
            if ui
                .input_text(field_label(ui, "Aircraft ICAO types"), &mut aircraft)
                .build()
            {
                mission.aircraft = aircraft
                    .split(',')
                    .map(|s| s.trim().to_uppercase())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            ui.input_text(field_label(ui, "Departure"), &mut mission.departure)
                .build();
            ui.input_text(field_label(ui, "Destination"), &mut mission.destination)
                .build();
            let mut minutes = mission.duration_minutes as i32;
            if ui
                .input_int(field_label(ui, "Approximate flight minutes"), &mut minutes)
                .build()
            {
                mission.duration_minutes = minutes.clamp(1, 720) as u32;
            }
            media(ui, &mut mission.media);
            for (index, event) in mission.events.iter_mut().enumerate() {
                let _id = ui.push_id_usize(index);
                if ui.collapsing_header(
                    format!("Event {} · {}", index + 1, event.id),
                    imgui::TreeNodeFlags::empty(),
                ) {
                    ui.input_text(field_label(ui, "Event ID"), &mut event.id)
                        .build();
                    let phases = [
                        "parked",
                        "clearance",
                        "pushback",
                        "taxi",
                        "departure",
                        "cruise",
                        "arrival",
                        "approach",
                        "landed",
                        "taxi_in",
                    ];
                    let mut p = phases.iter().position(|v| *v == event.phase).unwrap_or(0);
                    if ui.combo_simple_string(field_label(ui, "Flight phase"), &mut p, &phases) {
                        event.phase = phases[p].into();
                    }
                    let mut delay = event.delay_seconds as i32;
                    if ui
                        .input_int(field_label(ui, "Delay in phase (seconds)"), &mut delay)
                        .build()
                    {
                        event.delay_seconds = delay.clamp(0, 43200) as u32;
                    }
                    let mut probability = i32::from(event.probability);
                    if ui.slider(
                        field_label(ui, "Chance this flight (%)"),
                        0,
                        100,
                        &mut probability,
                    ) {
                        event.probability = probability as u8;
                    }
                    let levels = ["normal", "urgent", "emergency"];
                    let mut level = levels
                        .iter()
                        .position(|v| *v == event.severity)
                        .unwrap_or(0);
                    if ui.combo_simple_string(field_label(ui, "Urgency"), &mut level, &levels) {
                        event.severity = levels[level].into();
                    }
                    ui.input_text(field_label(ui, "Passenger seat"), &mut event.seat)
                        .build();
                    ui.input_text(
                        field_label(ui, "After event (optional)"),
                        &mut event.after_event,
                    )
                    .build();
                    ui.input_text(
                        field_label(ui, "Required outcome"),
                        &mut event.after_outcome,
                    )
                    .build();
                    lines(ui, "Attendant alternatives", &mut event.say);
                    for (i, choice) in event.choices.iter_mut().enumerate() {
                        let _choice = ui.push_id_usize(i);
                        ui.separator();
                        ui.input_text(field_label(ui, "Choice ID"), &mut choice.id)
                            .build();
                        ui.input_text(field_label(ui, "Captain's response"), &mut choice.label)
                            .build();
                        ui.input_text(field_label(ui, "Outcome"), &mut choice.outcome)
                            .build();
                        lines(ui, "Attendant acknowledgement", &mut choice.say);
                    }
                    media(ui, &mut event.media);
                }
            }
        }
        if ui.button("Validate and save") {
            let source = if self.advanced {
                self.source.clone()
            } else {
                toml::to_string_pretty(mission).unwrap_or_default()
            };
            match openatc_core::missions::parse(&source) {
                Ok(_) => engine.post("/missions/save", json!({"source":source})),
                Err(e) => self.notice = e,
            }
        }
        ui.text_wrapped("Advanced source also lets you add or remove events, choices and spoken aliases. Invalid drafts stay in the editor for correction. Media references are saved for future playback; this version uses the text briefing and spoken dialogue.");
    }
}

fn lines(ui: &imgui::Ui, label: &str, lines: &mut Vec<String>) {
    let mut text = lines.join("\n");
    if ui
        .input_text_multiline(
            field_label(ui, label),
            &mut text,
            [ui.content_region_avail()[0], 100.0],
        )
        .build()
    {
        *lines = text.lines().map(str::to_owned).collect();
    }
    widgets::muted_wrapped(ui, "One alternative per line. Slots: {seat}, {passengers}.");
}

fn field_label(ui: &imgui::Ui, label: &str) -> String {
    widgets::muted_wrapped(ui, label);
    ui.set_next_item_width(-1.0);
    format!("##{label}")
}

fn media(ui: &imgui::Ui, media: &mut Vec<Media>) {
    if ui.collapsing_header("Optional media references", imgui::TreeNodeFlags::empty()) {
        let mut remove = None;
        for (index, item) in media.iter_mut().enumerate() {
            let _id = ui.push_id_usize(index);
            let kinds = ["image", "audio", "video"];
            let mut kind = kinds.iter().position(|k| *k == item.kind).unwrap_or(0);
            if ui.combo_simple_string(field_label(ui, "Kind"), &mut kind, &kinds) {
                item.kind = kinds[kind].into();
            }
            ui.input_text(
                field_label(ui, "Relative path or HTTPS URL"),
                &mut item.source,
            )
            .build();
            ui.input_text_multiline(
                field_label(ui, "Caption or transcript"),
                &mut item.transcript,
                [ui.content_region_avail()[0], 60.0],
            )
            .build();
            if ui.small_button("Remove reference") {
                remove = Some(index);
            }
        }
        if let Some(index) = remove {
            media.remove(index);
        }
        if media.len() < 8 && ui.small_button("Add media reference") {
            media.push(Media {
                kind: "image".into(),
                ..Default::default()
            });
        }
    }
}
