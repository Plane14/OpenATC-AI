//! Preloaded cockpit callouts driven by observed indicated airspeed.
use openatc_audio::Speech;
use openatc_core::state::{PhaseCode, Telemetry};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    version: u32,
    enabled: bool,
    minimum_knots: f64,
    maximum_knots: f64,
    reset_below_knots: f64,
    maximum_agl_feet: f64,
    maximum_delay_seconds: f64,
    #[serde(default)]
    ias_refs: Vec<String>,
    v1_refs: Vec<String>,
    vr_refs: Vec<String>,
    v2_refs: Vec<String>,
    v1_text: String,
    vr_text: String,
    v2_text: String,
    #[serde(default)]
    v1_vr_text: Option<String>,
    #[serde(default)]
    log_labels: Vec<String>,
    #[serde(default)]
    log_template: Option<String>,
}
#[derive(Default)]
pub struct Runtime {
    audio: Speech,
    source: String,
    policy: Option<Policy>,
    refs: Vec<Vec<crate::panel::Readings>>,
    ias: Vec<crate::panel::Readings>,
    voice_key: String,
    fired: [bool; 3],
    pending: std::collections::VecDeque<(usize, std::time::Instant)>,
    playing: Option<usize>,
    previous: Option<f64>,
    rebound: Option<std::time::Instant>,
    status: String,
    pub active: bool,
}
impl Runtime {
    pub fn reset(&mut self) {
        self.audio.clear_preloaded();
        self.pending.clear();
        self.fired = [false; 3];
        self.previous = None;
        self.playing = None;
        self.active = false;
        self.voice_key.clear();
    }

    pub fn update(
        &mut self,
        data: &mut xplane::data::DataApi,
        aircraft: &str,
        interface: &mut openatc_ui::interface::Interface,
        telemetry: &Telemetry,
    ) {
        let path = std::path::Path::new(&interface.settings.simulator_root)
            .join("Resources/plugins/OpenATC/aircraft/callouts.toml");
        let source = format!("{aircraft}/{}", path.display());
        if self.source != source {
            self.source = source;
            self.policy = std::fs::read_to_string(&path)
                .ok()
                .or_else(|| Some(include_str!("../../../aircraft/callouts.toml").to_owned()))
                .and_then(|text| toml::from_str::<Policy>(&text).ok())
                .filter(|p| {
                    p.version == 1
                        && p.minimum_knots >= 20.0
                        && p.maximum_knots <= 350.0
                        && p.minimum_knots < p.maximum_knots
                        && p.reset_below_knots > 0.0
                        && p.reset_below_knots < p.minimum_knots
                        && (10.0..=2000.0).contains(&p.maximum_agl_feet)
                        && (0.1..=2.0).contains(&p.maximum_delay_seconds)
                        && [&p.v1_text, &p.vr_text, &p.v2_text]
                            .iter()
                            .all(|t| !t.is_empty() && t.len() < 64)
                });
            self.refs = self
                .policy
                .as_ref()
                .map(|p| {
                    [&p.v1_refs, &p.vr_refs, &p.v2_refs]
                        .into_iter()
                        .map(|names| {
                            names
                                .iter()
                                .take(8)
                                .filter_map(|name| crate::panel::find_readings(data, name))
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default();
            self.fired = [false; 3];
            self.previous = None;
            self.pending.clear();
            self.audio.clear_preloaded();
            self.voice_key.clear();
        }
        let Some(p) = self.policy.as_ref().filter(|p| p.enabled) else {
            self.audio.stop_speech();
            self.pending.clear();
            self.active = false;
            return;
        };
        if self.rebound.is_none_or(|at| at.elapsed().as_secs() >= 2) {
            self.rebound = Some(std::time::Instant::now());
            self.ias = p
                .ias_refs
                .iter()
                .take(8)
                .filter_map(|name| crate::panel::find_readings(data, name))
                .collect();
            self.refs = [&p.v1_refs, &p.vr_refs, &p.v2_refs]
                .into_iter()
                .map(|names| {
                    names
                        .iter()
                        .take(8)
                        .filter_map(|name| crate::panel::find_readings(data, name))
                        .collect()
                })
                .collect();
        }
        let settings = &interface.settings;
        let state = interface.engine.state();
        let key = format!(
            "{}/{}/{}/{}/{}/{:?}",
            interface.engine.endpoint(),
            settings.tts_url,
            settings.tts_model,
            settings.copilot_voice,
            settings.copilot_speed * settings.tts_speed,
            [
                p.v1_text.as_str(),
                p.vr_text.as_str(),
                p.v2_text.as_str(),
                p.v1_vr_text.as_deref().unwrap_or("")
            ]
        );
        if self.voice_key != key {
            self.audio.clear_preloaded();
            self.voice_key = key;
        }
        self.audio
            .configure("", &settings.output_device, 1.0, settings.copilot_volume);
        if telemetry.on_ground
            && telemetry.ground_speed_knots < p.reset_below_knots
            && state.phase != PhaseCode::Departure
        {
            self.fired = [false; 3];
            self.previous = None;
            self.pending.clear();
        }
        if !interface.engine.connected() || telemetry.paused {
            self.audio.stop_speech();
            self.pending.clear();
            self.active = false;
            return;
        }
        if telemetry.on_ground && matches!(state.phase, PhaseCode::Taxi | PhaseCode::Departure) {
            for (i, text) in [&p.v1_text, &p.vr_text, &p.v2_text].into_iter().enumerate() {
                self.audio.preload(
                    &i.to_string(),
                    &interface.engine.endpoint(),
                    text,
                    &settings.copilot_voice,
                    settings.copilot_speed * settings.tts_speed,
                );
            }
            if let Some(text) = p
                .v1_vr_text
                .as_ref()
                .filter(|text| !text.is_empty() && text.len() < 64)
            {
                self.audio.preload(
                    "3",
                    &interface.engine.endpoint(),
                    text,
                    &settings.copilot_voice,
                    settings.copilot_speed * settings.tts_speed,
                );
            }
        }
        let speeds: Vec<_> =
            self.refs
                .iter()
                .map(|refs| {
                    refs.iter().find_map(|read| {
                        read().first().copied().filter(|v| {
                            v.is_finite() && *v >= p.minimum_knots && *v <= p.maximum_knots
                        })
                    })
                })
                .collect();
        let speeds: [Option<f64>; 3] = speeds.try_into().unwrap_or([None; 3]);
        let valid = [(0, 1), (1, 2), (0, 2)]
            .iter()
            .all(|(a, b)| speeds[*a].zip(speeds[*b]).is_none_or(|(a, b)| a <= b));
        let status = format!(
            "V-speeds {:?}; cached {:?}; phase {:?}; ordered {valid}; fired {:?}; {}",
            speeds,
            [
                self.audio.preloaded("0"),
                self.audio.preloaded("1"),
                self.audio.preloaded("2")
            ],
            state.phase,
            self.fired,
            self.audio.status()
        );
        if self.status != status {
            crate::debug_log(&format!("OpenATC callouts: {status}\n"));
            self.status = status.clone();
        }
        interface.callout_status = status;
        self.active = state.phase == PhaseCode::Departure
            && telemetry.height_agl_feet < p.maximum_agl_feet
            && valid
            && speeds.iter().any(Option::is_some);
        if !self.active {
            self.audio.stop_speech();
            self.pending.clear();
            self.previous = None;
            return;
        }
        let Some(ias) = self
            .ias
            .iter()
            .find_map(|read| {
                read()
                    .first()
                    .copied()
                    .filter(|v| v.is_finite() && *v > 0.0)
            })
            .or(telemetry.indicated_speed_knots.filter(|v| v.is_finite()))
        else {
            return;
        };
        let crossed = crossings(self.previous, ias, speeds, self.fired);
        self.previous = Some(ias);
        if !self.audio.busy() {
            self.playing = None;
        }
        let rotation_clip = rotation_clip(
            &crossed,
            p.v1_vr_text
                .as_ref()
                .is_some_and(|text| !text.is_empty() && text.len() < 64),
        );
        let combined = rotation_clip == 3;
        for i in crossed {
            crate::debug_log(&format!(
                "OpenATC callout threshold: index={i} target={:?} IAS={ias:.1} AGL={:.1} busy={}\n",
                speeds[i],
                telemetry.height_agl_feet,
                self.audio.busy()
            ));
            self.fired[i] = true;
            if combined && i == 0 {
                continue;
            }
            if i == 1 {
                // Rotate takes precedence over V1; V2 waits briefly for rotation to finish.
                self.pending.retain(|(call, _)| *call != 0);
                if self.playing != Some(1) && self.audio.busy() {
                    self.audio.stop_speech();
                }
                self.pending
                    .push_front((rotation_clip, std::time::Instant::now()));
            } else {
                self.pending.push_back((i, std::time::Instant::now()));
            }
        }
        while self
            .pending
            .front()
            .is_some_and(|(_, at)| at.elapsed().as_secs_f64() > p.maximum_delay_seconds)
        {
            if let Some((index, at)) = self.pending.pop_front() {
                crate::debug_log(&format!(
                    "OpenATC callout expired: clip={index} queued_ms={} IAS={ias:.1}\n",
                    at.elapsed().as_millis()
                ));
            }
        }
        if let Some((i, queued)) = self.pending.front().copied()
            && self
                .audio
                .play_preloaded(&i.to_string(), settings.copilot_volume)
        {
            self.pending.pop_front();
            crate::debug_log(&format!(
                "OpenATC callout playback: clip={i} queued_ms={} IAS={ias:.1}\n",
                queued.elapsed().as_millis()
            ));
            self.playing = Some(i);
            let entries = log_entries(p, speeds, i);
            interface.engine.post(
                "/crew/callout",
                serde_json::json!({"entries":entries,"callsign":state.plan.callsign}),
            );
        }
    }
}
fn log_entries(policy: &Policy, speeds: [Option<f64>; 3], clip: usize) -> Vec<String> {
    let bundled: toml::Value = toml::from_str(include_str!("../../../aircraft/callouts.toml"))
        .expect("bundled callout policy");
    let template = policy
        .log_template
        .as_deref()
        .unwrap_or_else(|| bundled["log_template"].as_str().unwrap());
    let indices: &[usize] = if clip == 3 {
        &[0, 1]
    } else {
        std::slice::from_ref(&clip)
    };
    indices
        .iter()
        .filter_map(|index| {
            let speed = speeds.get(*index).copied().flatten()?;
            let label = policy
                .log_labels
                .get(*index)
                .map(String::as_str)
                .unwrap_or_else(|| bundled["log_labels"][*index].as_str().unwrap());
            Some(
                template
                    .replace("{callout}", label)
                    .replace("{speed}", &format!("{speed:.0}")),
            )
        })
        .collect()
}
fn rotation_clip(crossed: &[usize], combined_available: bool) -> usize {
    if combined_available && crossed.contains(&0) && crossed.contains(&1) {
        3
    } else {
        1
    }
}
fn crossings(
    previous: Option<f64>,
    current: f64,
    speeds: [Option<f64>; 3],
    fired: [bool; 3],
) -> Vec<usize> {
    let Some(previous) = previous.filter(|p| *p <= current) else {
        return vec![];
    };
    speeds
        .iter()
        .enumerate()
        .filter_map(|(i, speed)| {
            speed
                .filter(|speed| !fired[i] && previous < *speed && current >= *speed)
                .map(|_| i)
        })
        .collect()
}
#[cfg(test)]
mod tests {
    #[test]
    fn speed_policy_excludes_toliss_display_pointers_and_retains_stock_bugs() {
        let policy: super::Policy =
            toml::from_str(include_str!("../../../aircraft/callouts.toml")).unwrap();
        assert_eq!(policy.v1_refs[0], "toliss_airbus/performance/V1");
        assert_eq!(policy.vr_refs[0], "toliss_airbus/performance/VR");
        assert_eq!(policy.v2_refs[0], "toliss_airbus/performance/V2");
        assert!(!policy.v1_refs.iter().any(|name| name == "AirbusFBW/V1"));
        assert!(!policy.vr_refs.iter().any(|name| name == "AirbusFBW/VR"));
        for (index, refs) in [&policy.v1_refs, &policy.vr_refs, &policy.v2_refs]
            .into_iter()
            .enumerate()
        {
            assert!(refs.contains(&format!(
                "sim/cockpit2/gauges/actuators/airspeed_bugs[{index}]"
            )));
        }
        assert_eq!(
            super::crossings(Some(123.0), 125.0, [Some(124.0), None, None], [false; 3]),
            vec![0]
        );
    }
    #[test]
    fn reported_speeds_and_equal_rotation_keep_their_calls() {
        let speeds = [Some(127.0), Some(146.0), Some(148.0)];
        let mut fired = [false; 3];
        for (previous, current, expected) in
            [(126.0, 127.0, 0), (145.0, 146.0, 1), (147.0, 148.0, 2)]
        {
            let crossed = super::crossings(Some(previous), current, speeds, fired);
            assert_eq!(crossed, vec![expected]);
            fired[expected] = true;
        }
        assert!(super::crossings(Some(148.0), 149.0, speeds, fired).is_empty());
        let crossed = super::crossings(
            Some(145.0),
            146.0,
            [Some(146.0), Some(146.0), Some(148.0)],
            [false; 3],
        );
        assert_eq!(crossed, vec![0, 1]);
        assert_eq!(super::rotation_clip(&crossed, true), 3);
        assert_eq!(super::rotation_clip(&[1], true), 1);
        assert_eq!(super::rotation_clip(&crossed, false), 1);
        let policy: super::Policy =
            toml::from_str(include_str!("../../../aircraft/callouts.toml")).unwrap();
        assert_eq!(super::log_entries(&policy, speeds, 0), vec!["V1 · 127 kt"]);
        assert_eq!(
            super::log_entries(&policy, speeds, 1),
            vec!["Rotate · 146 kt"]
        );
        assert_eq!(super::log_entries(&policy, speeds, 2), vec!["V2 · 148 kt"]);
        assert_eq!(
            super::log_entries(&policy, [Some(146.0), Some(146.0), Some(148.0)], 3),
            vec!["V1 · 146 kt", "Rotate · 146 kt"]
        );
    }
    #[test]
    fn close_speeds_fire_once_and_never_on_a_late_attach_or_rejected_takeoff() {
        assert_eq!(
            super::crossings(Some(129.0), 132.0, [None, Some(132.0), None], [false; 3]),
            vec![1]
        );
        assert!(
            super::crossings(
                None,
                160.0,
                [Some(130.0), Some(132.0), Some(140.0)],
                [false; 3]
            )
            .is_empty()
        );
        assert_eq!(
            super::crossings(
                Some(129.0),
                132.1,
                [Some(130.0), Some(132.0), Some(140.0)],
                [false; 3]
            ),
            vec![0, 1]
        );
        assert!(
            super::crossings(
                Some(132.1),
                120.0,
                [Some(130.0), Some(132.0), Some(140.0)],
                [false; 3]
            )
            .is_empty()
        );
        assert!(
            super::crossings(
                Some(129.0),
                133.0,
                [Some(130.0), Some(132.0), Some(140.0)],
                [true, true, false]
            )
            .is_empty()
        );
    }
}
