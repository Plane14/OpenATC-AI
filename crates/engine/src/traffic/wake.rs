//! Observed liftoffs and departure wake delays, kept separate from radio chatter.
use crate::EngineState;
use openatc_core::{
    airport::Airport,
    state::{Telemetry, TrafficTarget},
    traffic::{
        runway_position,
        wake::{self, Departure},
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    time::Instant,
};

struct Liftoff {
    airport: String,
    runway: String,
    heading: f64,
    category: Option<char>,
    at: f64,
}

struct ArrivalWake {
    airport: String,
    runway: String,
    required_nm: f64,
}

#[derive(Default)]
pub(super) struct Monitor {
    clock: Option<f64>,
    wall: Option<Instant>,
    elapsed: f64,
    previous: BTreeMap<u32, TrafficTarget>,
    departures: VecDeque<Liftoff>,
    arrival: Option<ArrivalWake>,
}

impl Monitor {
    fn arrival_conflict(
        &mut self,
        t: &Telemetry,
        airport: Option<&Airport>,
        runway: &str,
        delta: f64,
    ) -> bool {
        let Some(airport) = airport.filter(|a| wake::applies(&a.icao)) else {
            self.arrival = None;
            return false;
        };
        let Some(own) =
            openatc_core::traffic::approach_position(airport, runway, t.latitude, t.longitude)
        else {
            self.arrival = None;
            return false;
        };
        if t.on_ground
            || own.0 > 0.0
            || own.1.abs() > 500.0
            || openatc_core::navigation::turn(
                t.ground_track_degrees.unwrap_or(t.heading_degrees),
                own.3,
            )
            .abs()
                > 30.0
        {
            self.arrival = None;
            return false;
        }
        if self.arrival.as_ref().is_some_and(|a| {
            a.airport != airport.icao || a.runway != runway || -own.0 / 1852.0 >= a.required_nm
        }) {
            self.arrival = None;
        }
        for target in t.traffic.as_ref().into_iter().flatten().take(126) {
            let Some(previous) = self
                .previous
                .get(&target.id)
                .filter(|p| !p.on_ground && p.callsign == target.callsign)
            else {
                continue;
            };
            let Some(leader_runway) = wake::approach_runway(airport, runway, target) else {
                continue;
            };
            let (Some(before), Some(after)) = (
                openatc_core::traffic::approach_position(
                    airport,
                    leader_runway,
                    previous.latitude,
                    previous.longitude,
                ),
                openatc_core::traffic::approach_position(
                    airport,
                    leader_runway,
                    target.latitude,
                    target.longitude,
                ),
            ) else {
                continue;
            };
            if before.0 >= 0.0
                || after.0 < 0.0
                || before.1.abs() > 500.0
                || after.1.abs() > 500.0
                || (target.altitude_feet - airport.elevation_feet) > 1000.0
                || openatc_core::navigation::turn(target.track_degrees, own.3).abs() > 30.0
            {
                continue;
            }
            let required_nm = wake::category(&target.aircraft)
                .zip(wake::category(&t.aircraft_icao))
                .and_then(|(lead, follow)| wake::approach_nm(lead, follow))
                .unwrap_or(10.0);
            if required_nm == 0.0 {
                continue;
            }
            let elapsed_since_threshold = delta.max(0.0) * after.0 / (after.0 - before.0);
            let Some(relative) = openatc_core::traffic::approach_position(
                airport,
                runway,
                target.latitude,
                target.longitude,
            ) else {
                continue;
            };
            let along = relative.0 - after.0 - own.0
                + t.ground_speed_knots * elapsed_since_threshold * 1852.0 / 3600.0;
            let spacing = if along < 0.0 {
                0.0
            } else {
                along.hypot(relative.1 - own.1) / 1852.0
            };
            if spacing < required_nm {
                self.arrival = Some(ArrivalWake {
                    airport: airport.icao.clone(),
                    runway: runway.into(),
                    required_nm,
                });
            }
        }
        self.arrival.is_some()
    }
}

pub(super) fn observe(state: &mut EngineState) {
    let t = &state.session.telemetry;
    let monitor = &mut state.traffic.wake;
    let now = Instant::now();
    let wall_delta = monitor
        .wall
        .replace(now)
        .map_or(0.0, |last| now.duration_since(last).as_secs_f64());
    let clock = t.simulation_seconds.filter(|v| v.is_finite() && *v >= 0.0);
    let delta = match (monitor.clock.replace(clock.unwrap_or(0.0)), clock) {
        (Some(before), Some(after)) => after - before,
        (_, None) => wall_delta,
        _ => 0.0,
    };
    if clock.is_none() {
        monitor.clock = None;
    }
    if !t.paused && (0.0..=30.0).contains(&delta) {
        monitor.elapsed += delta;
    }
    monitor
        .departures
        .retain(|d| monitor.elapsed - d.at < 240.0);
    if t.paused || !t.position_valid {
        return;
    }
    if !(0.0..=30.0).contains(&delta) {
        // A teleport or clock reset is not an observed liftoff or threshold crossing.
        monitor.previous.clear();
    }
    state.session.departure_wake_seconds = 0.0;
    state.session.arrival_wake_conflict = monitor.arrival_conflict(
        t,
        state.arrival_airport.as_ref(),
        &state.session.plan.arrival_runway,
        delta,
    );

    if let Some(targets) = &t.traffic {
        for target in targets.iter().take(126).filter(|p| p.id != 0) {
            let lifted = !target.on_ground
                && monitor
                    .previous
                    .get(&target.id)
                    .is_some_and(|p| p.on_ground && p.callsign == target.callsign);
            if !lifted || !openatc_core::traffic::advisory::valid(target) {
                continue;
            }
            for airport in state
                .airport
                .iter()
                .chain(state.arrival_airport.iter())
                .filter(|a| wake::applies(&a.icao))
            {
                if let Some((runway, heading)) = airport.runways.iter().find_map(|r| {
                    let p =
                        runway_position(airport, &r.first_name, target.latitude, target.longitude)?;
                    if !(-100.0..=p.2 + 1000.0).contains(&p.0)
                        || p.1.abs() > r.width / 2.0 + 50.0
                        || (target.altitude_feet - airport.elevation_feet) > 600.0
                    {
                        return None;
                    }
                    let first =
                        openatc_core::navigation::turn(p.3, target.track_degrees).abs() < 90.0;
                    Some((
                        if first {
                            r.first_name.clone()
                        } else {
                            r.second_name.clone()
                        },
                        if first { p.3 } else { (p.3 + 180.0) % 360.0 },
                    ))
                }) {
                    monitor.departures.push_back(Liftoff {
                        airport: airport.icao.clone(),
                        runway,
                        heading,
                        category: wake::category(&target.aircraft),
                        at: monitor.elapsed,
                    });
                    break;
                }
            }
        }
        monitor.previous = targets
            .iter()
            .take(126)
            .filter(|p| p.id != 0)
            .map(|p| (p.id, p.clone()))
            .collect();
    } else {
        monitor.previous.clear();
    }
    while monitor.departures.len() > 64 {
        monitor.departures.pop_front();
    }
    if !t.on_ground {
        return;
    }
    let Some(airport) = state
        .airport
        .as_ref()
        .filter(|a| a.icao == state.session.plan.departure)
    else {
        return;
    };
    let Some(own) = runway_position(airport, &state.session.plan.runway, t.latitude, t.longitude)
    else {
        return;
    };
    let follower = wake::category(if t.aircraft_icao.is_empty() {
        &state.session.plan.aircraft
    } else {
        &t.aircraft_icao
    });
    for event in &monitor.departures {
        if event.airport != airport.icao {
            continue;
        }
        if !wake::related_runways(
            airport,
            &event.runway,
            &state.session.plan.runway,
            event.category,
        ) {
            continue;
        }
        let kind = if openatc_core::navigation::turn(event.heading, own.3).abs() > 90.0 {
            Departure::Opposite
        } else if own.0 > 152.4 {
            Departure::Intersection
        } else {
            Departure::FullLength
        };
        let required = event
            .category
            .zip(follower)
            .and_then(|(a, b)| wake::departure_seconds(a, b, kind))
            .unwrap_or(240);
        let remaining = (f64::from(required) - (monitor.elapsed - event.at)).max(0.0);
        state.session.departure_wake_seconds = state.session.departure_wake_seconds.max(remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::{
        airport::{Airport, Runway},
        state::{Point, Telemetry},
    };
    #[test]
    fn touching_down_or_losing_the_leader_does_not_erase_a_threshold_wake_conflict() {
        for (spacing, conflict) in [(2.0, true), (5.0, false)] {
            let mut s = crate::test_state();
            s.session.plan.arrival_runway = "09".into();
            s.arrival_airport = Some(Airport {
                icao: "KTEST".into(),
                runways: vec![Runway {
                    first_name: "09".into(),
                    second_name: "27".into(),
                    width: 45.0,
                    second: Point {
                        east: 3000.0,
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            });
            s.session.telemetry = Telemetry {
                position_valid: true,
                aircraft_icao: "P28A".into(),
                on_ground: false,
                longitude: -spacing / 60.0,
                ground_speed_knots: 100.0,
                heading_degrees: 90.0,
                simulation_seconds: Some(10.0),
                traffic: Some(vec![TrafficTarget {
                    id: 2,
                    callsign: "TEST2".into(),
                    aircraft: "A320".into(),
                    longitude: -0.0001,
                    altitude_feet: 30.0,
                    track_degrees: 90.0,
                    on_ground: false,
                    ..Default::default()
                }]),
                ..Default::default()
            };
            observe(&mut s);
            let leader = &mut s.session.telemetry.traffic.as_mut().unwrap()[0];
            leader.longitude = 0.0001;
            leader.on_ground = true;
            s.session.telemetry.simulation_seconds = Some(11.0);
            observe(&mut s);
            assert_eq!(s.session.arrival_wake_conflict, conflict);
            s.session.telemetry.traffic = Some(vec![]);
            s.session.telemetry.simulation_seconds = Some(12.0);
            observe(&mut s);
            assert_eq!(s.session.arrival_wake_conflict, conflict);
            s.session.telemetry.heading_degrees = 0.0;
            observe(&mut s);
            assert!(!s.session.arrival_wake_conflict);
        }
    }

    #[test]
    fn threshold_observation_uses_the_leaders_displaced_parallel_threshold() {
        let mut s = crate::test_state();
        s.session.plan.arrival_runway = "09L".into();
        s.arrival_airport = Some(Airport {
            icao: "KTEST".into(),
            runways: vec![
                Runway {
                    first_name: "09L".into(),
                    second_name: "27R".into(),
                    width: 45.0,
                    second: Point {
                        east: 3000.0,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Runway {
                    first_name: "09R".into(),
                    second_name: "27L".into(),
                    width: 45.0,
                    first: Point {
                        north: 500.0,
                        ..Default::default()
                    },
                    second: Point {
                        east: 3000.0,
                        north: 500.0,
                        ..Default::default()
                    },
                    first_displaced_meters: 500.0,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        s.session.telemetry = Telemetry {
            position_valid: true,
            on_ground: false,
            aircraft_icao: "P28A".into(),
            longitude: -2.0 / 60.0,
            heading_degrees: 90.0,
            simulation_seconds: Some(1.0),
            traffic: Some(vec![TrafficTarget {
                id: 2,
                callsign: "TEST2".into(),
                aircraft: "A320".into(),
                latitude: 500.0 / 111320.0,
                longitude: -0.0001,
                track_degrees: 90.0,
                on_ground: false,
                ..Default::default()
            }]),
            ..Default::default()
        };
        observe(&mut s);
        s.session.telemetry.traffic.as_mut().unwrap()[0].longitude = 0.0001;
        s.session.telemetry.simulation_seconds = Some(2.0);
        observe(&mut s);
        assert!(
            !s.session.arrival_wake_conflict,
            "Physical pavement start is not the landing threshold"
        );
        s.session.telemetry.traffic.as_mut().unwrap()[0].longitude = 510.0 / 111320.0;
        s.session.telemetry.simulation_seconds = Some(7.0);
        observe(&mut s);
        assert!(s.session.arrival_wake_conflict);
    }
    #[test]
    fn observed_heavy_departure_delays_takeoff_and_pause_does_not_age_wake() {
        let mut s = crate::test_state();
        s.session.plan.departure = "KTEST".into();
        s.session.plan.runway = "09".into();
        s.airport = Some(Airport {
            icao: "KTEST".into(),
            runways: vec![Runway {
                first_name: "09".into(),
                second_name: "27".into(),
                width: 45.0,
                second: Point {
                    east: 3000.0,
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        });
        let target = TrafficTarget {
            id: 42,
            callsign: "TEST42".into(),
            aircraft: "B77W".into(),
            longitude: 0.01,
            track_degrees: 90.0,
            on_ground: true,
            ..Default::default()
        };
        s.session.telemetry = Telemetry {
            aircraft_icao: "A320".into(),
            position_valid: true,
            simulation_seconds: Some(10.0),
            traffic: Some(vec![target]),
            ..Default::default()
        };
        observe(&mut s);
        s.session.telemetry.simulation_seconds = Some(11.0);
        s.session.telemetry.traffic.as_mut().unwrap()[0].on_ground = false;
        observe(&mut s);
        assert_eq!(s.session.departure_wake_seconds, 120.0);
        s.session.telemetry.paused = true;
        s.session.telemetry.simulation_seconds = Some(200.0);
        observe(&mut s);
        s.session.telemetry.paused = false;
        observe(&mut s);
        assert_eq!(s.session.departure_wake_seconds, 120.0);
        for i in 1..=12 {
            s.session.telemetry.simulation_seconds = Some(200.0 + f64::from(i) * 10.0);
            observe(&mut s);
        }
        assert_eq!(s.session.departure_wake_seconds, 0.0);
    }
}
