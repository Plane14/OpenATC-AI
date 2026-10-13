//! User-requested diagnostic conversation export; excludes provider settings/secrets.
use openatc_core::state::State;
use std::fmt::Write as _;
use std::io::Write;

pub fn conversation(state: &State) -> String {
    let mut text = format!(
        "OpenATC conversation\nFlight: {} -> {} | callsign {} | runway {}\nPhase: {:?} | COM1 {:.3} MHz | radio power {}\nPosition: {:.6}, {:.6} | on ground {} | ground speed {:.2} kt | AGL {:.1} ft\n\n",
        state.plan.departure,
        state.plan.destination,
        state.plan.callsign,
        state.plan.runway,
        state.phase,
        f64::from(state.telemetry.com1_khz) / 1000.,
        state.telemetry.radio_power,
        state.telemetry.latitude,
        state.telemetry.longitude,
        state.telemetry.on_ground,
        state.telemetry.ground_speed_knots,
        state.telemetry.height_agl_feet
    );
    let _ = writeln!(
        text,
        "Altitude: geometric {:.1} ft | indicated {:?} ft | standard-pressure {:?} ft",
        state.telemetry.altitude_feet,
        state.telemetry.indicated_altitude_feet,
        state.telemetry.pressure_altitude_feet
    );
    let route = &state.taxi_clearance;
    let _ = writeln!(
        text,
        "Surface: {:?} | hold {} | crossing {} | route #{} | notified COM {} | waiting for traffic {}\nRadio: busy {} | sequence seen {:?} | handoff {} / sequence {}\nTraffic: {}\n",
        route.stage(),
        route.hold_short_runway,
        route.crossing_runway,
        route.sequence,
        route.holding_channel,
        route.waiting_for_traffic,
        state.telemetry.radio_busy,
        state.telemetry.radio_sequence_seen,
        state.recommended_frequency_khz,
        state.frequency_sequence,
        state
            .telemetry
            .traffic
            .as_ref()
            .map_or("unavailable".into(), |targets| format!(
                "{} reported",
                targets.len()
            ))
    );
    for entry in &state.transcript {
        let _ = writeln!(
            text,
            "[{}] {}{}: {}",
            entry.sequence,
            entry.speaker,
            if entry.position.is_empty() {
                String::new()
            } else {
                format!(" - {}", entry.position)
            },
            entry.text
        );
    }
    text
}

pub fn copy(text: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let commands: &[(&str, &[&str])] = &[
        ("wl-copy", &["--type", "text/plain;charset=utf-8"]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    #[cfg(target_os = "macos")]
    let commands: &[(&str, &[&str])] = &[("pbcopy", &[])];
    #[cfg(target_os = "windows")]
    let commands: &[(&str, &[&str])] = &[(
        "powershell",
        &["-NoProfile", "-Command", "$input | Set-Clipboard"],
    )];
    for (program, args) in commands {
        let Ok(mut child) = std::process::Command::new(program)
            .args(*args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        let written = child
            .stdin
            .take()
            .is_some_and(|mut input| input.write_all(text.as_bytes()).is_ok());
        if child.wait().is_ok_and(|status| status.success()) && written {
            return Ok(());
        }
    }
    Err("Could not access the desktop clipboard from the simulator session.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_includes_departure_mismatch_context_and_controller() {
        let mut state = State::default();
        state.plan.departure = "YMLT".into();
        state.plan.destination = "VLVT".into();
        state.telemetry.com1_khz = 118_100;
        state.transcript.push(serde_json::from_value(serde_json::json!({"speaker":"ATC", "position":"VIENTIANE Tower", "text":"Your flight departs YMLT."})).unwrap());
        let text = conversation(&state);
        assert!(text.contains("YMLT -> VLVT"));
        assert!(text.contains("118.100 MHz"));
        assert!(text.contains("ATC - VIENTIANE Tower: Your flight departs YMLT."));
    }
}
