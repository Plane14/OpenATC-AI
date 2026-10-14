//! Traffic actually reported by the simulator, including runway-blocking diagnostics.
use crate::widgets;
use openatc_core::state::State;

pub fn settings(ui: &imgui::Ui, interface: &mut crate::interface::Interface) {
    widgets::section_title(ui, "TRAFFIC SOURCE");
    ui.text_wrapped("OpenATC generated traffic is still in development. Native X-Plane AI is optional; installing Bluebell models alone does not create flights.");
    if let Some(reply) = interface.engine.take_reply("/traffic/models/index") {
        interface.traffic_notice = if reply.success {
            format!(
                "{} compatible models · {} aircraft types · {} operators · {} background CPU workers. {} catalogue warnings.",
                reply.data["models"],
                reply.data["types"].as_array().map_or(0, Vec::len),
                reply.data["operators"].as_array().map_or(0, Vec::len),
                reply.data["cpuWorkers"],
                reply.data["warnings"].as_array().map_or(0, Vec::len)
            )
        } else {
            reply.error
        };
    }
    if ui.collapsing_header("Advanced CPU budget", imgui::TreeNodeFlags::empty()) {
        let mut workers = i32::from(interface.settings.cpu_workers);
        widgets::muted_wrapped(ui, "Background CPU workers (0 = automatic)");
        ui.set_next_item_width(-1.0);
        if ui.slider("##cpu-workers", 0, 16, &mut workers) {
            interface.settings.cpu_workers = workers as u8;
        }
        ui.text_wrapped("Automatic uses up to half the available CPU threads, capped at four. One worker limits background CPU use. Changes apply when the engine restarts; simulator SDK calls stay on the simulator thread.");
    }
    {
        let _disabled = ui.begin_disabled(!interface.simulator_host);
        ui.checkbox("Suppress X-Plane AI traffic", &mut interface.settings.suppress_native_traffic);
        ui.text_wrapped("Keeps native AI out of OpenATC's traffic flow. Other traffic plugins retain ownership. X-Plane's Draw Parked Aircraft option is separate.");
        let _native_disabled = ui.begin_disabled(interface.settings.suppress_native_traffic);
        ui.checkbox(
            "Set X-Plane AI traffic amount",
            &mut interface.settings.manage_native_traffic,
        );
        let _count_disabled = ui.begin_disabled(!interface.settings.manage_native_traffic);
        let mut count = i32::from(interface.settings.native_traffic_count);
        widgets::muted_wrapped(ui, "AI aircraft");
        ui.set_next_item_width(-1.0);
        if ui.slider("##ai-aircraft", 0, 19, &mut count) {
            interface.settings.native_traffic_count = count as u8;
        }
    }
    ui.text_wrapped("Native AI uses X-Plane flight models. Start with a small count and measure FPS. Your existing aircraft models and simulator routes are used; another traffic provider always keeps priority.");
    if !interface.native_traffic_status.is_empty() {
        ui.text_wrapped(&interface.native_traffic_status);
    }
    widgets::section_title(ui, "TRAFFIC RADIO");
    ui.checkbox(
        "Hear simulated traffic pilot reports",
        &mut interface.settings.hear_traffic_radio,
    );
    ui.checkbox(
        "Talk to simulated traffic pilots",
        &mut interface.settings.talk_to_traffic,
    );
    ui.text_wrapped("Pilot conversations use observed simulator traffic and weather. They are simulated replies, not communications with multiplayer pilots. Runway and taxi protection remain active with traffic voices off.");
    if ui.collapsing_header(
        "Bluebell model installation",
        imgui::TreeNodeFlags::DEFAULT_OPEN,
    ) {
        model_installer(ui, interface);
        if ui.button("Scan installed model catalogue") {
            interface
                .engine
                .post("/traffic/models/index", serde_json::json!({"refresh":true}));
        }
    }
    if ui.collapsing_header(
        "Airline, hub and aircraft preferences",
        imgui::TreeNodeFlags::empty(),
    ) {
        profiles(ui, interface);
    }
    if !interface.traffic_notice.is_empty() {
        ui.text_wrapped(&interface.traffic_notice);
    }
}

pub fn tab(ui: &imgui::Ui, interface: &mut crate::interface::Interface, state: &State) {
    if ui.button("Traffic settings & model downloads") {
        interface.show_settings_section(8);
    }
    if let Some(reply) = interface.engine.take_reply("/traffic/talk") {
        interface.traffic_notice = if reply.success {
            String::new()
        } else {
            reply.error
        };
    }
    if !interface.traffic_notice.is_empty() {
        ui.text_wrapped(&interface.traffic_notice);
    }
    ui.text_wrapped("OpenATC checks simulator-reported aircraft before issuing runway crossing, takeoff and landing clearances.");
    let Some(targets) = state.telemetry.traffic.as_ref() else {
        ui.text_colored(widgets::warning(ui), "Traffic feed unavailable");
        ui.text_wrapped("Runway checks cannot confirm clear traffic. Check the simulator connection and the traffic provider's TCAS output.");
        return;
    };
    ui.text(format!("{} aircraft reported", targets.len()));
    if state.taxi_clearance.waiting_for_traffic {
        ui.text_colored(
            widgets::warning(ui),
            format!(
                "Waiting to enter runway {}",
                state.taxi_clearance.hold_short_runway
            ),
        );
    }
    ui.spacing();
    ui.text_wrapped("Lightweight OpenATC-controlled CSL traffic is still in development. Native AI does not use Bluebell models or accept OpenATC flight plans.");
    ui.text_wrapped("Bluebell contains visual CSL aircraft models; it does not generate or control flights by itself.");
    ui.spacing();
    widgets::section_title(ui, "NEARBY AIRCRAFT");
    if let Some(_table) = ui.begin_table_with_flags(
        "TrafficTargets",
        5,
        imgui::TableFlags::ROW_BG
            | imgui::TableFlags::BORDERS_INNER_H
            | imgui::TableFlags::SORTABLE
            | imgui::TableFlags::SIZING_STRETCH_PROP,
    ) {
        for column in [
            "Aircraft",
            "Distance (NM)",
            "Altitude (ft MSL)",
            "Track",
            "Position",
        ] {
            ui.table_setup_column(column);
        }
        ui.table_headers_row();
        let mut rows: Vec<_> = targets
            .iter()
            .map(|t| {
                (
                    openatc_core::ops::distance_nm(
                        state.telemetry.latitude,
                        state.telemetry.longitude,
                        t.latitude,
                        t.longitude,
                    ),
                    t,
                )
            })
            .collect();
        let (column, descending) = ui
            .table_sort_specs_mut()
            .and_then(|s| {
                s.specs().iter().next().map(|sort| {
                    (
                        sort.column_idx(),
                        sort.sort_direction() == Some(imgui::TableSortDirection::Descending),
                    )
                })
            })
            .unwrap_or((0, false));
        rows.sort_by(|a, b| {
            let order = match column {
                0 => a.1.callsign.cmp(&b.1.callsign),
                2 => a.1.altitude_feet.total_cmp(&b.1.altitude_feet),
                3 => a.1.track_degrees.total_cmp(&b.1.track_degrees),
                4 => a.1.on_ground.cmp(&b.1.on_ground),
                _ => a.0.total_cmp(&b.0),
            };
            if descending { order.reverse() } else { order }
        });
        for (distance, target) in rows {
            ui.table_next_row();
            ui.table_next_column();
            let _id = ui.push_id_int(target.id as i32);
            if target.callsign.is_empty() {
                ui.text_disabled("Unidentified");
            } else if ui
                .selectable_config(&target.callsign)
                .selected(interface.traffic_selected == target.id)
                .build()
            {
                interface.traffic_selected = target.id;
            }
            ui.table_next_column();
            ui.text(format!("{distance:.1}"));
            ui.table_next_column();
            ui.text(format!("{:.0}", target.altitude_feet));
            ui.table_next_column();
            ui.text(format!("{:03.0}°", target.track_degrees));
            ui.table_next_column();
            ui.text(if target.on_ground {
                "On ground"
            } else {
                "Airborne"
            });
        }
    }
    if let Some(target) = targets
        .iter()
        .find(|t| t.id != 0 && t.id == interface.traffic_selected)
    {
        ui.separator();
        ui.text(format!("Talk to {} · {}", target.callsign, target.aircraft));
        let _disabled =
            ui.begin_disabled(!interface.settings.talk_to_traffic || state.telemetry.radio_busy);
        for (label, topic) in [
            ("Ask about weather", "weather"),
            ("Ask about landing", "landing"),
            ("Ask about crosswind", "crosswind"),
        ] {
            if ui.button(label) {
                interface.engine.post(
                    "/traffic/talk",
                    serde_json::json!({"id":target.id,"topic":topic}),
                );
            }
        }
        widgets::muted_wrapped(ui, "Message this pilot");
        ui.set_next_item_width(-1.0);
        widgets::edit_text(
            ui,
            "##traffic-message",
            &mut interface.traffic_message,
            1000,
        );
        let _chat_disabled = ui.begin_disabled(
            !interface.settings.informal_pilot_chat || interface.traffic_message.trim().is_empty(),
        );
        if ui.button("Send to pilot") {
            interface.engine.post(
                "/traffic/talk",
                serde_json::json!({"id":target.id,"message":interface.traffic_message}),
            );
            interface.traffic_message.clear();
        }
        ui.text_wrapped("Enable informal conversation under Realism for free text. This talks to a simulated pilot, not a multiplayer user.");
    }
}

fn profiles(ui: &imgui::Ui, interface: &mut crate::interface::Interface) {
    if let Some(reply) = interface.engine.take_reply("/traffic/profiles") {
        interface.traffic_profiles = if reply.success {
            reply.data
        } else {
            serde_json::json!({"errors":[reply.error]})
        };
    }
    if ui.button("Load / validate profile files") {
        interface
            .engine
            .post("/traffic/profiles", serde_json::json!({"refresh":true}));
        interface.traffic_profiles = serde_json::json!({"loading":true});
    }
    if interface.traffic_profiles["loading"] == true {
        ui.text("Reading traffic preferences...");
    }
    ui.text_wrapped("Edit traffic/airlines.toml, hubs.toml and aircraft.toml to change the simulated mix. Preferences are approximate, not live schedules. Invalid edits are rejected and the last valid set is kept.");
    let p = &interface.traffic_profiles;
    if let Some(errors) = p["errors"].as_array() {
        for error in errors {
            ui.text_colored(
                widgets::warning(ui),
                error.as_str().unwrap_or("Profile error"),
            );
        }
    }
    if let Some(airlines) = p["airlines"].as_object() {
        ui.text(format!(
            "{} fleets · {} airport preferences · {} aircraft types",
            airlines.len(),
            p["hubs"].as_object().map_or(0, serde_json::Map::len),
            p["aircraft"].as_object().map_or(0, serde_json::Map::len)
        ));
        if let Some(_table) = ui.begin_table_with_flags(
            "AirlineProfiles",
            3,
            imgui::TableFlags::ROW_BG | imgui::TableFlags::SIZING_STRETCH_PROP,
        ) {
            for heading in ["Operator", "Activity", "Aircraft types"] {
                ui.table_setup_column(heading);
            }
            ui.table_headers_row();
            for airline in airlines.values() {
                ui.table_next_row();
                ui.table_next_column();
                ui.text_wrapped(airline["name"].as_str().unwrap_or(""));
                ui.table_next_column();
                ui.text(airline["category"].as_str().unwrap_or("").replace('_', " "));
                ui.table_next_column();
                ui.text_wrapped(
                    airline["fleet"]
                        .as_object()
                        .map(|v| v.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default(),
                );
            }
        }
    }
}

fn model_installer(ui: &imgui::Ui, interface: &mut crate::interface::Interface) {
    for path in [
        "/traffic/models/status",
        "/traffic/models/install",
        "/traffic/models/cancel",
    ] {
        if let Some(reply) = interface.engine.take_reply(path) {
            if path == "/traffic/models/status" {
                interface.model_install_pending = false;
            }
            if !reply.success {
                interface.model_install_status["stage"] = "failed".into();
                interface.model_install_status["error"] = reply.error.into();
            } else if path != "/traffic/models/cancel" {
                interface.model_install_status = reply.data;
            }
        }
    }
    let now = ui.time();
    if interface.engine.connected()
        && !interface.model_install_pending
        && (interface.model_install_checked == 0.0 || now - interface.model_install_checked >= 0.5)
    {
        interface.model_install_pending = true;
        interface.model_install_checked = now;
        interface
            .engine
            .post("/traffic/models/status", serde_json::json!({}));
    }
    let status = &interface.model_install_status;
    let stage = status["stage"].as_str().unwrap_or("");
    let busy = matches!(
        stage,
        "preparing" | "downloading" | "extracting" | "installing" | "cancelling"
    );
    widgets::section_title(ui, "BLUEBELL AIRCRAFT MODELS");
    if busy {
        ui.text(format!(
            "{} — archive {} of {}",
            match stage {
                "downloading" => "Downloading",
                "extracting" => "Extracting",
                "installing" => "Installing",
                "cancelling" => "Cancelling",
                _ => "Preparing download",
            },
            (status["completed"].as_u64().unwrap_or(0) + 1)
                .min(status["archiveCount"].as_u64().unwrap_or(12)),
            status["archiveCount"].as_u64().unwrap_or(12)
        ));
        ui.text(status["archive"].as_str().unwrap_or(""));
        let downloaded = status["downloaded"].as_u64().unwrap_or(0) as f64 / 1_048_576.0;
        let total = status["total"].as_u64().unwrap_or(0) as f64 / 1_048_576.0;
        if total > 0.0 {
            imgui::ProgressBar::new((downloaded / total).min(1.0) as f32)
                .size([ui.content_region_avail()[0], 0.0])
                .overlay_text(format!("{downloaded:.1} / {total:.1} MiB"))
                .build(ui);
        } else {
            ui.text(format!("{downloaded:.1} MiB downloaded"));
        }
        ui.text(format!(
            "{} files extracted",
            status["files"].as_u64().unwrap_or(0)
        ));
        if ui.button("Cancel download") {
            interface
                .engine
                .post("/traffic/models/cancel", serde_json::json!({}));
        }
    } else if stage == "installed" {
        ui.text_colored(
            widgets::accent(ui),
            format!(
                "Bluebell installed — {} model objects in {} packages",
                status["objects"].as_u64().unwrap_or(0),
                status["packages"].as_u64().unwrap_or(0)
            ),
        );
        ui.text_wrapped(status["path"].as_str().unwrap_or(""));
    } else {
        if stage == "failed" || stage == "cancelled" {
            ui.text_wrapped(status["error"].as_str().unwrap_or("Download cancelled."));
        }
        let _disabled = ui.begin_disabled(!interface.engine.connected());
        if ui.button(if stage.is_empty() {
            "Download and install Bluebell"
        } else {
            "Retry Bluebell installation"
        }) {
            interface
                .engine
                .post("/traffic/models/install", serde_json::json!({}));
            interface.model_install_status =
                serde_json::json!({"stage":"preparing","archiveCount":12});
        }
    }
    ui.text_wrapped("Downloads the publisher's DDS model packs and installs them in OpenATC's CSL folder. Allow several GB of disk space. Completed downloads are kept for retry if interrupted. Models retain their original licences.");
    ui.spacing();
}
