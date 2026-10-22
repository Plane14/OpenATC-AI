//! Developer setup controls. Work and database access belong to the companion engine.
use crate::interface::Interface;

pub(crate) fn controls(ui: &imgui::Ui, interface: &mut Interface) {
    if let Some(reply) = interface.engine.take_reply("/setup/status") {
        interface.setup_pending = false;
        if reply.success {
            interface.setup_status = reply.data;
        } else {
            interface.setup_status["error"] = reply.error.into();
        }
    }
    if let Some(reply) = interface.engine.take_reply("/setup/rebuild") {
        if !reply.success {
            interface.setup_status["error"] = reply.error.into();
            interface.setup_status["setup"]["running"] = false.into();
        }
        interface.setup_checked = -1.0;
    }
    let now = ui.time();
    if interface.engine.connected()
        && (!interface.setup_pending || now - interface.setup_checked > 10.0)
        && (interface.setup_checked < 0.0 || now - interface.setup_checked >= 1.0)
    {
        interface.setup_pending = true;
        interface.setup_checked = now;
        interface
            .engine
            .post("/setup/status", serde_json::json!({}));
    }
    ui.separator();
    ui.text_wrapped("Re-setup rebuilds the TOML and scenery caches from your current files. Your editable TOML, settings, flight plans and logs are kept. Scenery is rebuilt as needed. Restart the plugin after completion to reload all active policies.");
    let running = interface.setup_status["setup"]["running"]
        .as_bool()
        .unwrap_or(false);
    {
        let _disabled = ui.begin_disabled(!interface.engine.connected() || running);
        if ui.button("Re-setup") {
            interface.setup_status["setup"]["running"] = true.into();
            interface.setup_status["setup"]["phase"] = "Waiting for setup worker".into();
            interface.setup_status["error"] = serde_json::Value::Null;
            interface
                .engine
                .post("/setup/rebuild", serde_json::json!({}));
        }
    }
    if !interface.engine.connected() {
        ui.text_wrapped("Connect to the companion engine to re-run setup.");
    }
    let status = &interface.setup_status;
    if let Some(phase) = status["setup"]["phase"].as_str() {
        ui.text_wrapped(phase);
    }
    if let Some(errors) = status["setup"]["errors"].as_array() {
        for error in errors {
            if let Some(text) = error.as_str() {
                ui.text_wrapped(text);
            }
        }
    }
    for error in [&status["error"], &status["toml"]["error"]] {
        if let Some(text) = error.as_str().filter(|s| !s.is_empty()) {
            ui.text_wrapped(text);
        }
    }
    ui.text(format!(
        "TOML cache: {} reused | {} parsed | {} saved",
        status["toml"]["hits"].as_u64().unwrap_or(0),
        status["toml"]["misses"].as_u64().unwrap_or(0),
        status["toml"]["writes"].as_u64().unwrap_or(0)
    ));
    if let Some(path) = status["toml"]["path"].as_str() {
        ui.text_wrapped(path);
    }
}
