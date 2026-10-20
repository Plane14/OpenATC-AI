//! Generated surface exchanges use the same readback templates as the player.
use super::*;
use openatc_core::{
    apply::add_transmission,
    dialogue::say,
    state::{SpeechTag, State},
    stations::Station,
};

#[derive(Clone)]
pub(super) struct Exchange {
    pub(super) id: u32,
    pub(super) segment: usize,
    pub(super) station: Station,
    pub(super) callsign: String,
    pub(super) messages: Vec<(bool, String)>,
    pub(super) next: usize,
    pub(super) due: Instant,
    pub(super) grant: bool,
    pub(super) tune_after: Option<Station>,
    pub(super) sector_transfer: Option<(Arc<super::flight::Prepared>, usize)>,
    /// Recovery reports cannot complete an exchange from the original route.
    pub(super) recovery_from: Option<usize>,
    pub(super) rejoin: bool,
}

pub(super) fn hold_takeoff(mut exchange: Exchange, cancelled: bool, wake: bool) -> Exchange {
    let slots = [("callsign", exchange.callsign.clone())];
    exchange.messages = vec![
        (
            true,
            say(
                match (cancelled, wake) {
                    (true, true) => "generated_takeoff_wake_cancelled",
                    (false, true) => "generated_takeoff_wake_hold",
                    (true, false) => "generated_takeoff_traffic_cancelled",
                    (false, false) => "generated_takeoff_traffic_hold",
                },
                &slots,
            ),
        ),
        (false, say("pilot_traffic_hold", &slots)),
    ];
    exchange.next = 0;
    exchange.due = Instant::now();
    exchange.grant = false;
    exchange.tune_after = None;
    exchange
}

pub(super) fn hold_landing(mut exchange: Exchange, cancelled: bool) -> Exchange {
    let slots = [("callsign", exchange.callsign.clone())];
    exchange.messages = vec![
        (
            true,
            say(
                if cancelled {
                    "generated_landing_cancelled"
                } else {
                    "arrival_continue_no_landing"
                },
                &slots,
            ),
        ),
        (false, say("generated_landing_withheld_readback", &slots)),
    ];
    exchange.next = 0;
    exchange.due = Instant::now();
    exchange.grant = false;
    exchange.tune_after = None;
    exchange
}

pub(super) fn hold_airborne(mut exchange: Exchange) -> Exchange {
    let slots = [("callsign", exchange.callsign.clone())];
    exchange.messages = vec![
        (true, say("generated_airborne_clearance_withheld", &slots)),
        (
            false,
            say("generated_airborne_clearance_withheld_readback", &slots),
        ),
    ];
    exchange.next = 0;
    exchange.due = Instant::now();
    exchange.grant = false;
    exchange.tune_after = None;
    exchange
}

pub(super) fn cancel_crossing(mut exchange: Exchange, runway: Option<&str>) -> Exchange {
    let slots = [("callsign", exchange.callsign.clone())];
    let instruction = runway.map_or_else(
        || say("generated_taxi_hold", &slots),
        |runway| {
            say(
                "generated_crossing_cancelled",
                &[
                    ("callsign", exchange.callsign.clone()),
                    ("runway", runway.to_owned()),
                ],
            )
        },
    );
    exchange.messages = vec![
        (true, instruction),
        (false, say("pilot_traffic_hold", &slots)),
    ];
    exchange.next = 0;
    exchange.grant = false;
    exchange.tune_after = None;
    exchange.due = Instant::now();
    exchange
}

fn station(state: &EngineState, airport: &Airport) -> Option<Station> {
    state
        .stations
        .iter()
        .find(|station| station.airport == airport.icao && station.service == "Ground")
        .cloned()
        .or_else(|| {
            airport
                .frequencies
                .iter()
                .find(|frequency| frequency.service == "Ground")
                .map(|frequency| Station {
                    airport: airport.icao.clone(),
                    airport_name: airport.name.clone(),
                    name: frequency.name.clone(),
                    service: "Ground".into(),
                    khz: frequency.khz,
                    latitude: airport.reference_latitude,
                    longitude: airport.reference_longitude,
                    elevation_feet: airport.elevation_feet,
                    ..Default::default()
                })
        })
        .or_else(|| {
            state
                .stations
                .iter()
                .find(|station| {
                    station.airport == airport.icao
                        && station.service == "Tower"
                        && station.services.iter().any(|service| service == "Ground")
                })
                .cloned()
        })
}

fn departure_hold(state: &mut EngineState) {
    for pose in
        state.generated.world.snapshot().into_iter().filter(|pose| {
            pose.complete && pose.stage == openatc_core::traffic::motion::Stage::TaxiOut
        })
    {
        if !state.generated.departure_ready(pose.id) {
            continue;
        }
        let Some(airport) = state.generated.departure_airport(pose.id) else {
            continue;
        };
        let Some(tower) = state
            .stations
            .iter()
            .find(|station| station.airport == airport.icao && station.service == "Tower")
            .cloned()
            .or_else(|| {
                airport
                    .frequencies
                    .iter()
                    .find(|frequency| frequency.service == "Tower")
                    .map(|frequency| Station {
                        airport: airport.icao.clone(),
                        airport_name: airport.name.clone(),
                        name: frequency.name.clone(),
                        service: "Tower".into(),
                        khz: frequency.khz,
                        latitude: airport.reference_latitude,
                        longitude: airport.reference_longitude,
                        elevation_feet: airport.elevation_feet,
                        ..Default::default()
                    })
            })
        else {
            continue;
        };
        let Some(ground) = station(state, &airport) else {
            continue;
        };
        if state.generated.tower_ready.contains(&pose.id) {
            if let Some(prepared) = state.generated.flights.get(&pose.id).cloned()
                && let Err(error) = super::flight::install(&mut state.generated, pose.id, &prepared)
                && state.generated.warnings.len() < 128
            {
                state.generated.warnings.push(error);
                state.generated.flights.remove(&pose.id);
            }
            continue;
        }
        let Some(frame) = state
            .generated
            .frames
            .iter()
            .find(|frame| frame.target.id == pose.id)
        else {
            continue;
        };
        let callsign = frame.target.callsign.clone();
        let Some(runway) = state
            .generated
            .orders
            .get(&pose.id)
            .and_then(|orders| orders.last())
            .map(|(_, order)| order.hold_short_runway.clone())
            .filter(|runway| !runway.is_empty())
        else {
            continue;
        };
        let mut messages = Vec::new();
        let (speaker, tune_after) =
            if !state.generated.tower_notified.contains(&pose.id) && ground.khz != tower.khz {
                let slots = [
                    ("callsign", callsign.clone()),
                    ("station", tower.name.clone()),
                    ("frequency", format!("{:.3}", f64::from(tower.khz) / 1000.0)),
                ];
                messages.push((true, say("flight_handoff", &slots)));
                messages.push((false, say("pilot_frequency_readback", &slots)));
                state.generated.tower_notified.insert(pose.id);
                (ground.clone(), Some(tower.clone()))
            } else {
                messages.push((
                    false,
                    say(
                        "pilot_request_ready",
                        &[
                            ("callsign", callsign.clone()),
                            ("station", tower.name.clone()),
                        ],
                    ),
                ));
                messages.push((
                    true,
                    say(
                        "generated_departure_standby",
                        &[("callsign", callsign.clone()), ("runway", runway.clone())],
                    ),
                ));
                messages.push((
                    false,
                    say(
                        "generated_hold_short_readback",
                        &[("callsign", callsign.clone()), ("runway", runway)],
                    ),
                ));
                state.generated.tower_ready.insert(pose.id);
                (tower.clone(), Some(tower.clone()))
            };
        state.generated.exchange = Some(Exchange {
            id: pose.id,
            segment: pose.segment,
            station: speaker,
            callsign,
            messages,
            next: 0,
            due: Instant::now(),
            grant: false,
            tune_after,
            sector_transfer: None,
            recovery_from: None,
            rejoin: false,
        });
        break;
    }
}

fn blocked(state: &EngineState, id: u32, order: &openatc_core::state::TaxiClearance) -> bool {
    if order.crossing_runway.is_empty() {
        return false;
    }
    let Some(airport) = state.generated.surface_airport(&order.airport) else {
        return true;
    };
    // Generated conflicts are held by the movement world; only observed traffic can add a
    // runway-occupancy conflict here.
    let targets = super::external_targets(&state.generated, &state.session.telemetry);
    let Some(policy) = &state.generated.policy else {
        return true;
    };
    let Some(segment) = state.generated.world.current_segment(id) else {
        return true;
    };
    let distance = segment
        .points
        .windows(2)
        .map(|pair| {
            openatc_core::arrival::distance_nm(
                [pair[0].latitude, pair[0].longitude],
                [pair[1].latitude, pair[1].longitude],
            ) * 1852.0
        })
        .sum::<f64>();
    let window = distance / (segment.knots.max(1.0) * 0.514444)
        + order.instructions.chars().count() as f64 * 2.0 / policy.radio_characters_per_second
        + policy.radio_gap_seconds * 2.0
        + policy.runway_clearance_buffer_seconds;
    openatc_core::holding::blocked_during(
        airport,
        &order.crossing_runway,
        targets.iter().filter(|target| target.id != id),
        window,
    )
    .is_some()
}

pub fn tick(state: &mut EngineState) {
    super::flight::synchronise_recoveries(&mut state.generated);
    super::ctaf::announce(state);
    if !state.settings.generated_traffic
        || state.generated.epoch != Some(state.session_generation)
        || state.generated.simulation_job.is_some()
        || state.generated.preparation_job.is_some()
        || state.generated.turnaround_job.is_some()
        || state.session.telemetry.paused
        || state.session.telemetry.radio_busy
        || state.pending_copilot_reply.is_some()
        || state
            .session
            .transcript
            .iter()
            .rev()
            .find(|entry| entry.is_controller_instruction())
            .is_some_and(|entry| !entry.pilot_reply.is_empty())
    {
        return;
    }
    let Some((rate, gap)) = state
        .generated
        .policy
        .as_ref()
        .map(|policy| (policy.radio_characters_per_second, policy.radio_gap_seconds))
    else {
        return;
    };
    if state.generated.exchange.is_none() {
        super::flight::prepare_limit_rejoins(&mut state.generated);
        state.generated.exchange = super::flight::exchange(state);
    }
    if state.generated.exchange.is_none() {
        for pose in state.generated.world.snapshot().into_iter().filter(|pose| {
            pose.awaiting_clearance
                && state
                    .generated
                    .approved_orders
                    .contains(&(pose.id, pose.segment))
        }) {
            if !state.generated.departure_ready(pose.id) {
                continue;
            }
            let Some(order) = state
                .generated
                .orders
                .get(&pose.id)
                .and_then(|orders| orders.iter().find(|(segment, _)| *segment == pose.segment))
                .map(|(_, order)| order.clone())
            else {
                continue;
            };
            if order.crossing_runway.is_empty() || !blocked(state, pose.id, &order) {
                continue;
            }
            let Some(airport) = state.generated.surface_airport(&order.airport) else {
                continue;
            };
            let Some(station) = station(state, airport) else {
                continue;
            };
            let Some(frame) = state
                .generated
                .frames
                .iter()
                .find(|frame| frame.target.id == pose.id)
            else {
                continue;
            };
            let callsign = frame.target.callsign.clone();
            let occupied = cells(
                airport,
                openatc_core::airport::airport_point(
                    airport,
                    pose.position.latitude,
                    pose.position.longitude,
                ),
                frame.target.wingspan_meters.unwrap_or(80.0) / 2.0,
                state
                    .generated
                    .policy
                    .as_ref()
                    .unwrap()
                    .conflict_cell_meters,
            );
            if !state.generated.world.cancel_reservation(
                state.generated.world.generation(),
                pose.id,
                &occupied,
            ) {
                continue;
            }
            state
                .generated
                .approved_orders
                .remove(&(pose.id, pose.segment));
            state.generated.exchange = Some(Exchange {
                id: pose.id,
                segment: pose.segment,
                station,
                callsign: callsign.clone(),
                messages: vec![
                    (
                        true,
                        say(
                            "generated_crossing_cancelled",
                            &[
                                ("callsign", callsign.clone()),
                                ("runway", order.crossing_runway),
                            ],
                        ),
                    ),
                    (false, say("pilot_traffic_hold", &[("callsign", callsign)])),
                ],
                next: 0,
                due: Instant::now(),
                grant: false,
                tune_after: None,
                sector_transfer: None,
                recovery_from: None,
                rejoin: false,
            });
            break;
        }
    }
    if state.generated.exchange.is_none() {
        for pose in state
            .generated
            .world
            .snapshot()
            .into_iter()
            .filter(|pose| pose.awaiting_clearance)
        {
            if !state.generated.departure_ready(pose.id) {
                continue;
            }
            if state
                .generated
                .approved_orders
                .contains(&(pose.id, pose.segment))
            {
                continue;
            }
            if pose.stage == openatc_core::traffic::motion::Stage::Pushback {
                let Some(airport) = state.generated.departure_airport(pose.id) else {
                    continue;
                };
                let Some(station) = station(state, &airport) else {
                    continue;
                };
                let Some(frame) = state
                    .generated
                    .frames
                    .iter()
                    .find(|frame| frame.target.id == pose.id)
                else {
                    continue;
                };
                let callsign = frame.target.callsign.clone();
                let occupied = external_surface_cells(&state.generated, &state.session.telemetry);
                if !state.generated.world.reserve_surface_route(
                    state.generated.world.generation(),
                    pose.id,
                    pose.segment,
                    &occupied,
                ) {
                    continue;
                }
                let slots = [
                    ("callsign", callsign.clone()),
                    ("station", station.name.clone()),
                ];
                if let Some(frame) = state
                    .generated
                    .frames
                    .iter_mut()
                    .find(|frame| frame.target.id == pose.id)
                {
                    frame.target.com1_khz = Some(station.khz);
                }
                state.generated.exchange = Some(Exchange {
                    id: pose.id,
                    segment: pose.segment,
                    station,
                    callsign,
                    messages: vec![
                        (false, say("pilot_request_start_pushback", &slots)),
                        (true, say("generated_start_pushback_approved", &slots)),
                        (false, say("pilot_start_pushback_ack", &slots)),
                    ],
                    next: 0,
                    due: Instant::now(),
                    grant: true,
                    tune_after: None,
                    sector_transfer: None,
                    recovery_from: None,
                    rejoin: false,
                });
                break;
            }
            let Some(order) = state
                .generated
                .orders
                .get(&pose.id)
                .and_then(|orders| orders.iter().find(|(segment, _)| *segment == pose.segment))
                .map(|(_, order)| order.clone())
            else {
                continue;
            };
            if blocked(state, pose.id, &order) {
                continue;
            }
            let Some(airport) = state.generated.surface_airport(&order.airport) else {
                continue;
            };
            let Some(station) = station(state, airport) else {
                continue;
            };
            let Some(frame) = state
                .generated
                .frames
                .iter()
                .find(|frame| frame.target.id == pose.id)
            else {
                continue;
            };
            let callsign = frame.target.callsign.clone();
            let mut actor = State::default();
            actor.plan.callsign.clone_from(&callsign);
            actor.taxi_clearance = order.clone();
            actor.taxi_clearance.pending_readback = true;
            add_transmission(
                &mut actor,
                "ATC",
                &order.instructions,
                &SpeechTag {
                    position: station.name.clone(),
                    ..Default::default()
                },
            );
            let Some(reply) = openatc_core::readback::auto_reply(&actor) else {
                continue;
            };
            let occupied = external_surface_cells(&state.generated, &state.session.telemetry);
            if !state.generated.world.reserve_surface_route(
                state.generated.world.generation(),
                pose.id,
                pose.segment,
                &occupied,
            ) {
                if order.crossing_runway.is_empty()
                    && state
                        .generated
                        .reported_holds
                        .insert((pose.id, pose.segment))
                {
                    let blocker = state
                        .generated
                        .world
                        .surface_blocker(pose.id)
                        .and_then(|id| {
                            state
                                .generated
                                .frames
                                .iter()
                                .find(|frame| frame.target.id == id)
                        })
                        .map(|frame| frame.target.callsign.clone());
                    let text = if let Some(traffic) = blocker {
                        say(
                            "generated_taxi_hold_traffic",
                            &[("callsign", callsign.clone()), ("traffic", traffic)],
                        )
                    } else {
                        say("generated_taxi_hold", &[("callsign", callsign.clone())])
                    };
                    state.generated.exchange = Some(Exchange {
                        id: pose.id,
                        segment: pose.segment,
                        station,
                        callsign: callsign.clone(),
                        messages: vec![
                            (true, text),
                            (false, say("pilot_traffic_hold", &[("callsign", callsign)])),
                        ],
                        next: 0,
                        due: Instant::now(),
                        grant: false,
                        tune_after: None,
                        sector_transfer: None,
                        recovery_from: None,
                        rejoin: false,
                    });
                    break;
                }
                continue;
            }
            let mut messages = Vec::new();
            if let Some((segment, previous)) =
                state.generated.orders.get(&pose.id).and_then(|orders| {
                    orders.iter().rev().find(|(segment, order)| {
                        *segment < pose.segment && !order.crossing_runway.is_empty()
                    })
                })
                && !state
                    .generated
                    .reported_crossings
                    .contains(&(pose.id, *segment))
            {
                messages.push((
                    false,
                    say(
                        "pilot_runway_vacated",
                        &[
                            ("callsign", callsign.clone()),
                            ("runway", previous.crossing_runway.clone()),
                        ],
                    ),
                ));
                state
                    .generated
                    .reported_crossings
                    .insert((pose.id, *segment));
            }
            if order.crossing_runway.is_empty()
                && state.generated.orders.get(&pose.id).is_some_and(|orders| {
                    orders
                        .first()
                        .is_some_and(|(segment, _)| *segment == pose.segment)
                })
            {
                messages.push((
                    false,
                    say(
                        "pilot_request_taxi",
                        &[
                            ("callsign", callsign.clone()),
                            ("station", station.name.clone()),
                        ],
                    ),
                ));
            }
            messages.push((
                true,
                say(
                    "generated_surface_instruction",
                    &[
                        ("callsign", callsign.clone()),
                        ("instructions", order.instructions),
                    ],
                ),
            ));
            messages.push((false, reply.text));
            for frame in &mut state.generated.frames {
                if frame.target.id == pose.id {
                    frame.target.com1_khz = Some(station.khz);
                }
            }
            state.generated.exchange = Some(Exchange {
                id: pose.id,
                segment: pose.segment,
                station,
                callsign,
                messages,
                next: 0,
                due: Instant::now(),
                grant: true,
                tune_after: None,
                sector_transfer: None,
                recovery_from: None,
                rejoin: false,
            });
            break;
        }
    }
    if state.generated.exchange.is_none() {
        state.generated.exchange = super::flight::exchange(state);
    }
    if state.generated.exchange.is_none() {
        departure_hold(state);
    }
    let Some(mut exchange) = state.generated.exchange.take() else {
        return;
    };
    if exchange.sector_transfer.as_ref().is_some_and(|(plan, _)| {
        !state
            .generated
            .flights
            .get(&exchange.id)
            .is_some_and(|current| Arc::ptr_eq(plan, current))
    }) {
        return;
    }
    if exchange.due > Instant::now() {
        state.generated.exchange = Some(exchange);
        return;
    }
    if exchange.next == exchange.messages.len() {
        if exchange.rejoin {
            if let Some(receiver) = exchange.tune_after.as_ref() {
                if let Some(frame) = state
                    .generated
                    .frames
                    .iter_mut()
                    .find(|frame| frame.target.id == exchange.id)
                {
                    frame.target.com1_khz = Some(receiver.khz);
                    state
                        .generated
                        .controllers
                        .insert(exchange.id, receiver.clone());
                }
            } else if exchange.grant
                && let Some(source) = exchange.recovery_from
            {
                if super::flight::rejoin_blocked(
                    &state.generated,
                    &state.session.telemetry,
                    exchange.id,
                ) {
                    state.generated.exchange = Some(hold_airborne(exchange));
                    return;
                }
                super::flight::accept_rejoin(&mut state.generated, exchange.id, source);
            }
            return;
        }
        if exchange.grant
            && state.generated.active_flights.contains(&exchange.id)
            && state
                .generated
                .flights
                .get(&exchange.id)
                .and_then(|plan| plan.clearances.get(&exchange.segment))
                .is_some_and(|clearance| {
                    super::separation::blocked(
                        &state.generated,
                        &state.session.telemetry,
                        exchange.id,
                        exchange.segment,
                        clearance,
                    )
                })
        {
            state
                .generated
                .reported_holds
                .insert((exchange.id, exchange.segment));
            let plan = &state.generated.flights[&exchange.id];
            if plan
                .route
                .segment(exchange.segment)
                .is_some_and(|s| s.stage == openatc_core::traffic::motion::Stage::Takeoff)
            {
                state.generated.exchange = Some(hold_takeoff(exchange, true, false));
            } else if plan.clearances[&exchange.segment].landing_rollout.is_some() {
                state.generated.exchange = Some(hold_landing(exchange, true));
            } else {
                state.generated.exchange = Some(hold_airborne(exchange));
            }
            return;
        }
        if exchange.grant
            && state
                .generated
                .flights
                .get(&exchange.id)
                .and_then(|plan| plan.clearances.get(&exchange.segment))
                .is_some_and(|clearance| clearance.landing_rollout.is_some())
            && state.generated.crossing_blocked(
                exchange.id,
                exchange.segment,
                &super::external_targets(&state.generated, &state.session.telemetry),
            )
        {
            super::flight::revoke_landing(&mut state.generated, exchange.id, exchange.segment);
            state.generated.exchange = Some(hold_landing(exchange, true));
            return;
        }
        if exchange.grant
            && state.generated.crossing_blocked(
                exchange.id,
                exchange.segment,
                &super::external_targets(&state.generated, &state.session.telemetry),
            )
            && state
                .generated
                .flights
                .get(&exchange.id)
                .and_then(|flight| flight.route.segment(exchange.segment))
                .is_some_and(|segment| {
                    segment.stage == openatc_core::traffic::motion::Stage::Takeoff
                })
        {
            state
                .generated
                .reported_holds
                .insert((exchange.id, exchange.segment));
            let wake = state
                .generated
                .departure_wake
                .get(&exchange.id)
                .is_some_and(|seconds| *seconds > 0.0);
            state.generated.exchange = Some(hold_takeoff(exchange, true, wake));
            return;
        }
        if exchange.grant
            && state.generated.active_flights.contains(&exchange.id)
            && state
                .generated
                .flights
                .get(&exchange.id)
                .and_then(|flight| flight.route.segment(exchange.segment))
                .is_some_and(|segment| {
                    segment.stage == openatc_core::traffic::motion::Stage::Crossing
                })
            && state.generated.crossing_blocked(
                exchange.id,
                exchange.segment,
                &super::external_targets(&state.generated, &state.session.telemetry),
            )
        {
            let runway = state
                .generated
                .flights
                .get(&exchange.id)
                .and_then(|flight| flight.vacated_reports.get(&(exchange.segment + 1)))
                .map(|report| report.runway.as_str());
            state.generated.exchange = Some(cancel_crossing(exchange, runway));
            return;
        }
        if let Some((plan, _)) = exchange.sector_transfer.as_ref()
            && (!state
                .generated
                .flights
                .get(&exchange.id)
                .is_some_and(|current| Arc::ptr_eq(plan, current))
                || state
                    .generated
                    .world
                    .snapshot()
                    .iter()
                    .any(|pose| pose.id == exchange.id && pose.recovering))
        {
            return;
        }
        if let Some(receiver) = exchange.tune_after.as_ref()
            && let Some(frame) = state
                .generated
                .frames
                .iter_mut()
                .find(|frame| frame.target.id == exchange.id)
        {
            frame.target.com1_khz = Some(receiver.khz);
            state
                .generated
                .controllers
                .insert(exchange.id, receiver.clone());
            if let Some((plan, count)) = exchange.sector_transfer.as_ref() {
                if state
                    .generated
                    .flights
                    .get(&exchange.id)
                    .is_some_and(|current| Arc::ptr_eq(plan, current))
                    && state
                        .generated
                        .world
                        .snapshot()
                        .iter()
                        .any(|pose| pose.id == exchange.id && !pose.recovering)
                {
                    state
                        .generated
                        .sector_transfers
                        .insert((exchange.id, exchange.segment), *count);
                    state
                        .generated
                        .pending_sector_checkins
                        .insert(exchange.id, (plan.clone(), receiver.clone()));
                }
            } else if state.generated.active_flights.contains(&exchange.id) {
                state
                    .generated
                    .handoff_targets
                    .insert(exchange.id, exchange.segment);
                state
                    .generated
                    .pending_checkins
                    .insert((exchange.id, exchange.segment));
            }
        }
        if exchange.grant {
            state
                .generated
                .approved_orders
                .insert((exchange.id, exchange.segment));
            super::flight::acknowledge(&mut state.generated, exchange.id, exchange.segment);
        }
        return;
    }
    let (controller, text) = &exchange.messages[exchange.next];
    let spoken_text = state.generated.spoken_text(text);
    if state.settings.hear_traffic_radio
        && state.session.telemetry.radio_power
        && let Some(radio) = state.session.receiving_radio(exchange.station.khz)
    {
        let mut tag = if *controller {
            crate::flight::tag(state, &exchange.station)
        } else {
            SpeechTag {
                position: "Traffic".into(),
                voice: crate::traffic::pilot_voice(state, exchange.id),
                ..Default::default()
            }
        };
        tag.radio = Some(radio);
        let spoken_callsign = state.generated.spoken_text(&exchange.callsign);
        add_transmission(
            &mut state.session,
            if *controller { "ATC" } else { &spoken_callsign },
            &spoken_text,
            &tag,
        );
        state.session.transcript.last_mut().unwrap().background = true;
    }
    exchange.due =
        Instant::now() + Duration::from_secs_f64(spoken_text.chars().count() as f64 / rate + gap);
    exchange.next += 1;
    state.generated.exchange = Some(exchange);
}
