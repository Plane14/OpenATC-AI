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
    v1_refs: Vec<String>,
    vr_refs: Vec<String>,
    v2_refs: Vec<String>,
    v1_text: String,
    vr_text: String,
    v2_text: String,
}
#[derive(Default)]
pub struct Runtime {
    audio: Speech,
    source: String,
    policy: Option<Policy>,
    refs: Vec<Vec<crate::panel::Readings>>,
    voice_key: String,
    fired: [bool; 3],
    pending: std::collections::VecDeque<(usize, std::time::Instant)>,
    playing: Option<usize>,
    previous: Option<f64>,
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
        let settings = &interface.settings;
        let state = interface.engine.state();
        let key = format!(
            "{}/{}/{}/{}/{}/{:?}",
            interface.engine.endpoint(),
            settings.tts_url,
            settings.tts_model,
            settings.copilot_voice,
            settings.copilot_speed * settings.tts_speed,
            [p.v1_text.as_str(), p.vr_text.as_str(), p.v2_text.as_str()]
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
        let Some(ias) = telemetry.indicated_speed_knots.filter(|v| v.is_finite()) else {
            return;
        };
        let crossed = crossings(self.previous, ias, speeds, self.fired);
        self.previous = Some(ias);
        if !self.audio.busy() {
            self.playing = None;
        }
        for i in crossed {
            self.fired[i] = true;
            if i == 1 {
                // Rotate takes precedence over V1; V2 waits briefly for rotation to finish.
                self.pending.retain(|(call, _)| *call != 0);
                if self.playing != Some(1) && self.audio.busy() {
                    self.audio.stop_speech();
                }
                self.pending.push_front((i, std::time::Instant::now()));
            } else {
                self.pending.push_back((i, std::time::Instant::now()));
            }
        }
        while self
            .pending
            .front()
            .is_some_and(|(_, at)| at.elapsed().as_secs_f64() > p.maximum_delay_seconds)
        {
            self.pending.pop_front();
        }
        if let Some((i, _)) = self.pending.front().copied()
            && self
                .audio
                .play_preloaded(&i.to_string(), settings.copilot_volume)
        {
            self.pending.pop_front();
            self.playing = Some(i);
        }
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
