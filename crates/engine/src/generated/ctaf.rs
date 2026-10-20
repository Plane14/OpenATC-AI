//! Advisory-only traffic at untowered fields, selected and flown without a controller.
//!
//! A CTAF field publishes a common traffic advisory frequency and no Tower service, so the
//! traffic there makes its own position calls. The advisory procedure is the authority for
//! that traffic: the movement world grants its segments, never a controller clearance, and
//! the calls are spoken on the published frequency.
use super::*;
use openatc_core::{
    apply::add_transmission,
    state::SpeechTag,
    traffic::ctaf::{self, Circuit, Field, Pattern},
};
use std::time::{Duration, Instant};

/// Advisory aircraft published at one untowered field.
const MAXIMUM_ADVISORY_AIRCRAFT: usize = 2;

/// Identity range reserved for advisory traffic, clear of the generated fleet.
const ADVISORY_ID_BASE: u32 = 0x0e00_0000;

/// One advisory aircraft flying a published pattern at an untowered field.
#[derive(Clone)]
pub(super) struct Plan {
    /// Aircraft identity.
    pub(super) id: u32,
    /// Callsign making the advisories.
    pub(super) callsign: String,
    /// The field the pattern is flown at.
    pub(super) field: Field,
    /// The pattern, which produces the circuits.
    pub(super) pattern: Arc<Pattern>,
    /// The circuit currently being flown.
    pub(super) circuit: Circuit,
    /// Segments of the current circuit whose advisory has already been made.
    pub(super) announced: BTreeSet<usize>,
    /// When the next advisory may be made.
    pub(super) due: Option<Instant>,
    /// The go-around call, made instead of the final call when the runway is occupied.
    pub(super) go_around: Option<openatc_core::traffic::ctaf::Advisory>,
}

impl Plan {
    /// The advisory the aircraft makes on its present segment, when it makes one.
    fn advisory(&self, segment: usize) -> Option<&openatc_core::traffic::ctaf::Advisory> {
        self.circuit.advisory(segment)
    }
}

/// Slots left for advisory aircraft in a scene that already holds `used` aircraft.
fn advisory_capacity(used: usize) -> usize {
    MAXIMUM_AIRCRAFT
        .saturating_sub(used)
        .min(MAXIMUM_ADVISORY_AIRCRAFT)
}

/// Untwowered fields with an advisory frequency near the player, nearest first.
pub(super) fn fields(input: &Preparation, policy: &Operations) -> Vec<String> {
    let mut candidates = ctaf::candidates(&input.stations)
        .into_iter()
        .filter_map(|id| {
            let station = input.stations.iter().find(|station| station.airport == id)?;
            let distance = openatc_core::arrival::distance_nm(
                input.own,
                [station.latitude, station.longitude],
            );
            (distance.is_finite() && distance <= policy.network_radius_nm)
                .then_some((distance, id))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    candidates
        .into_iter()
        .map(|(_, id)| id)
        .take(policy.network_candidate_limit)
        .collect()
}

/// The longest runway at a field.
fn longest_runway(airport: &Airport) -> Option<String> {
    airport
        .runways
        .iter()
        .max_by(|a, b| {
            let length = |runway: &openatc_core::airport::Runway| {
                (runway.second.east - runway.first.east)
                    .hypot(runway.second.north - runway.first.north)
            };
            length(a).total_cmp(&length(b))
        })
        .map(|runway| runway.first_name.clone())
}

/// A light aircraft with an installed model that can use the field's runway.
fn select_aircraft(
    input: &Preparation,
    runway_meters: f64,
    seed: u64,
) -> Option<(String, String, String)> {
    let mut choices = Vec::new();
    for (id, airline) in &input.profiles.airlines {
        if airline.category != Category::GeneralAviation {
            continue;
        }
        for (kind, weight) in &airline.fleet {
            let Some(aircraft) = input.profiles.aircraft.get(kind) else {
                continue;
            };
            if runway_meters < aircraft.runway {
                continue;
            }
            let model = std::iter::once(kind.clone())
                .chain(aircraft.model_aliases.iter().cloned())
                .find_map(|code| {
                    input
                        .models
                        .choose(&code, &airline.operator, "", seed)
                        .map(|model| model.id.clone())
                });
            let Some(model) = model else {
                continue;
            };
            choices.push((*weight as u64, kind.clone(), model, id.clone()));
        }
    }
    if choices.is_empty() {
        return None;
    }
    choices.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let index = (seed % choices.len() as u64) as usize;
    choices.into_iter().nth(index).map(|(_, kind, model, airline)| {
        (kind, model, airline)
    })
}

/// Plan advisory traffic for one untowered field.
fn plan(
    input: &Preparation,
    airport: &Airport,
    seed: u64,
    identity: u32,
) -> Result<(Plan, GeneratedTraffic, Option<animation::Kind>), String> {
    let field = ctaf::field(airport, &input.stations)?;
    let runway = longest_runway(airport).ok_or("advisory field has no runway")?;
    let runway_meters = airport
        .runways
        .iter()
        .find(|candidate| {
            candidate.first_name == runway || candidate.second_name == runway
        })
        .map(|candidate| {
            (candidate.second.east - candidate.first.east)
                .hypot(candidate.second.north - candidate.first.north)
        })
        .unwrap_or(0.0);
    let (kind, model, airline) = select_aircraft(input, runway_meters, seed)
        .ok_or("no light aircraft has an installed model")?;
    let aircraft = input
        .profiles
        .aircraft
        .get(&kind)
        .ok_or("advisory traffic has no movement envelope")?;
    let installed = input
        .models
        .models
        .iter()
        .find(|candidate| candidate.id == model)
        .cloned()
        .ok_or("advisory traffic has no installed model")?;
    let callsign = input.profiles.airlines[&airline]
        .registration(&airport.icao, seed)
        .ok_or("advisory traffic has no registration pattern")?;
    let pattern = Pattern::new(airport, &runway, &callsign, field.clone())?;
    let circuit = pattern.circuit().clone();
    let start = circuit.route().start_position();
    let track = circuit
        .route()
        .segment(0)
        .and_then(|segment| segment.points.windows(2).next())
        .map_or(0.0, |pair| {
            openatc_core::arrival::bearing(
                [pair[0].latitude, pair[0].longitude],
                [pair[1].latitude, pair[1].longitude],
            )
        });
    let frame = GeneratedTraffic {
        vertical_speed_fpm: 0.0,
        body_heading_degrees: Some(track),
        target: TrafficTarget {
            id: identity,
            callsign: callsign.clone(),
            name: input.profiles.airlines[&airline].name.clone(),
            aircraft: kind.clone(),
            latitude: start.latitude,
            longitude: start.longitude,
            altitude_feet: start.altitude_feet,
            track_degrees: track,
            on_ground: false,
            ground_speed_knots: Some(90.0),
            wingspan_meters: Some(aircraft.wingspan),
            com1_khz: Some(field.frequency_khz),
            ..Default::default()
        },
        model: Arc::new(installed),
        animation: Animation::default(),
    };
    Ok((
        Plan {
            id: identity,
            callsign,
            field,
            pattern: Arc::new(pattern),
            circuit,
            announced: BTreeSet::new(),
            due: None,
            go_around: None,
        },
        frame,
        aircraft.handling,
    ))
}

/// Publish advisory traffic at the untowered fields near the player.
pub(super) fn prepare(scene: &mut Scene, input: &Preparation) {
    if input.counts[3] == 0 || input.simulator_root.is_empty() {
        return;
    }
    let (handling, policy, _) = match policy::load(&input.config) {
        Ok(loaded) => loaded,
        Err(error) => {
            scene.warnings.push(format!("advisory traffic: {error}"));
            return;
        }
    };
    let generation = scene.world.generation();
    let used = scene.flights.len().max(scene.frames.len());
    let capacity = advisory_capacity(used);
    if capacity == 0 {
        return;
    }
    let mut seed = input.epoch.wrapping_add(7);
    for id in fields(input, &policy) {
        if scene.advisories.len() >= capacity {
            break;
        }
        let airport = match super::airport(&input.simulator_root, &id) {
            Ok(airport) => airport,
            Err(error) => {
                scene
                    .warnings
                    .push(format!("{id}: advisory field: {error}"));
                continue;
            }
        };
        seed = seed.wrapping_mul(16777619) ^ (seed >> 32);
        let identity =
            ADVISORY_ID_BASE + u32::try_from(scene.advisories.len() + 1).unwrap_or(1);
        if scene
            .frames
            .iter()
            .any(|frame| frame.target.id == identity)
            || input
                .observed
                .iter()
                .any(|target| target.id == identity)
        {
            continue;
        }
        let (plan, frame, kind) = match plan(input, &airport, seed, identity) {
            Ok(planned) => planned,
            Err(error) => {
                scene
                    .warnings
                    .push(format!("{id}: advisory traffic: {error}"));
                continue;
            }
        };
        if !scene
            .world
            .insert_advisory(generation, identity, plan.circuit.route(), 0)
        {
            scene
                .warnings
                .push(format!("{id}: advisory traffic could not be admitted"));
            continue;
        }
        if let Some(kind) = kind
            && let Some(movement) = handling.get(&kind)
        {
            scene.movement.insert(identity, movement.clone());
        }
        scene.advisories.insert(identity, plan);
        scene.frames.push(frame);
    }
}

/// Grant the next segment of every advisory procedure and continue the finished circuits.
pub(super) fn advance(runtime: &mut Runtime, blocked: &BTreeSet<String>) {
    if runtime.advisories.is_empty() {
        return;
    }
    let generation = runtime.world.generation();
    let poses = runtime.world.snapshot();
    for pose in &poses {
        if !runtime.world.is_advisory(pose.id) {
            continue;
        }
        if pose.awaiting_clearance {
            runtime
                .world
                .authorise_advisory(generation, pose.id, pose.segment, blocked);
        }
    }
    // An occupied runway is never landed on: the aircraft goes around instead.
    for pose in &poses {
        if !runtime.world.is_advisory(pose.id) {
            continue;
        }
        let occupied = runtime
            .advisories
            .get(&pose.id)
            .is_some_and(|plan| runway_occupied(runtime, plan, blocked));
        if occupied {
            go_around(runtime, pose.id, pose.position);
        }
    }
    // A finished circuit is replaced by the next one, from the exact point it ended.
    let finished = runtime
        .world
        .snapshot()
        .into_iter()
        .filter(|pose| pose.complete && runtime.world.is_advisory(pose.id))
        .map(|pose| pose.id)
        .collect::<Vec<_>>();
    for id in finished {
        let Some(plan) = runtime.advisories.get(&id) else {
            continue;
        };
        let Ok(circuit) = plan.pattern.next_circuit() else {
            continue;
        };
        if !runtime
            .world
            .continue_advisory(generation, id, circuit.route())
        {
            continue;
        }
        if let Some(plan) = runtime.advisories.get_mut(&id) {
            plan.circuit = circuit;
            plan.announced.clear();
            plan.go_around = None;
        }
    }
}

/// Whether the runway the pattern uses is held by another aircraft.
fn runway_occupied(
    runtime: &Runtime,
    plan: &Plan,
    blocked: &BTreeSet<String>,
) -> bool {
    let resource = plan.pattern.runway_resource();
    if blocked.contains(resource) {
        return true;
    }
    runtime
        .world
        .resource_owner(resource)
        .is_some_and(|owner| owner != plan.id)
}

/// Turn an aircraft away from an occupied runway and make the go-around call.
fn go_around(runtime: &mut Runtime, id: u32, position: openatc_core::traffic::motion::Waypoint) {
    let Some(plan) = runtime.advisories.get(&id) else {
        return;
    };
    let Some(leg) = plan
        .circuit
        .legs()
        .iter()
        .find(|leg| leg.advisory.leg == "final")
    else {
        return;
    };
    let Some(pose) = runtime
        .world
        .snapshot()
        .into_iter()
        .find(|pose| pose.id == id)
    else {
        return;
    };
    if pose.segment != leg.segment {
        return;
    }
    let Ok(circuit) = plan.pattern.go_around_circuit(position) else {
        return;
    };
    let generation = runtime.world.generation();
    if !runtime.world.redirect_advisory(generation, id, circuit.route()) {
        return;
    }
    if let Some(plan) = runtime.advisories.get_mut(&id) {
        plan.circuit = circuit;
        plan.announced.clear();
        plan.go_around = Some(plan.pattern.go_around());
    }
}

/// Speak the advisories made by the aircraft that are established on a leg.
pub(super) fn announce(state: &mut EngineState) {
    if state.generated.advisories.is_empty() {
        return;
    }
    let poses = state.generated.world.snapshot();
    let mut pending = Vec::new();
    for (id, plan) in &state.generated.advisories {
        let Some(pose) = poses.iter().find(|pose| pose.id == *id) else {
            continue;
        };
        if plan.due.is_some_and(|due| Instant::now() < due) {
            continue;
        }
        // A go-around call is made instead of the call for the leg the aircraft abandoned.
        if let Some(advisory) = &plan.go_around {
            pending.push((
                *id,
                None,
                plan.callsign.clone(),
                advisory.text.clone(),
                plan.field.frequency_khz,
            ));
            continue;
        }
        if plan.announced.contains(&pose.segment) {
            continue;
        }
        let Some(advisory) = plan.advisory(pose.segment) else {
            continue;
        };
        pending.push((
            *id,
            Some(pose.segment),
            plan.callsign.clone(),
            advisory.text.clone(),
            plan.field.frequency_khz,
        ));
    }
    let Some((id, segment, callsign, text, frequency)) = pending.into_iter().next() else {
        return;
    };
    // The call is made on the leg the aircraft is established on, whether or not anybody is
    // listening; a later arrival on the frequency does not repeat it.
    let spoken_text = state.generated.spoken_text(&text);
    let rate = state
        .generated
        .policy
        .as_ref()
        .map_or(12.0, |policy| policy.radio_characters_per_second);
    let gap = state
        .generated
        .policy
        .as_ref()
        .map_or(0.4, |policy| policy.radio_gap_seconds);
    if let Some(plan) = state.generated.advisories.get_mut(&id) {
        match segment {
            Some(segment) => {
                plan.announced.insert(segment);
            }
            None => {
                plan.go_around = None;
            }
        }
        plan.due = Some(
            Instant::now()
                + Duration::from_secs_f64(spoken_text.chars().count() as f64 / rate + gap),
        );
    }
    if !state.settings.hear_traffic_radio || !state.session.telemetry.radio_power {
        return;
    }
    let Some(radio) = state.session.receiving_radio(frequency) else {
        return;
    };
    let mut tag = SpeechTag {
        position: "Traffic".into(),
        voice: crate::traffic::pilot_voice(state, id),
        ..Default::default()
    };
    tag.radio = Some(radio);
    let spoken_callsign = state.generated.spoken_text(&callsign);
    add_transmission(&mut state.session, &spoken_callsign, &spoken_text, &tag);
    if let Some(entry) = state.session.transcript.last_mut() {
        entry.background = true;
        entry.information_only = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openatc_core::traffic::csl::{Match, Model};

    fn station(airport: &str, service: &str, khz: i32, latitude: f64) -> openatc_core::stations::Station {
        openatc_core::stations::Station {
            airport: airport.into(),
            service: service.into(),
            khz,
            latitude,
            longitude: -90.0,
            ..Default::default()
        }
    }

    fn preparation(stations: Vec<openatc_core::stations::Station>) -> Preparation {
        let mut models = Catalogue::default();
        models.models.push(Model {
            id: "bluebell/c172".into(),
            objects: Vec::new(),
            vertical_offset: None,
            matches: vec![Match {
                aircraft: "C172".into(),
                operator: String::new(),
                livery: String::new(),
            }],
        });
        Preparation {
            stations: Arc::from(stations),
            regions: BTreeMap::new(),
            simulator_root: String::new(),
            origin: Airport::default(),
            destination: Airport::default(),
            profiles: Arc::new(Profiles::builtin().unwrap()),
            models: Arc::new(models),
            counts: [0, 0, 0, 2],
            own: [35.0, -90.0],
            observed: Vec::new(),
            reserved_stand_cells: BTreeSet::new(),
            reserved_surface_corridors: BTreeSet::new(),
            epoch: 1,
            runway: String::new(),
            arrival_runway: String::new(),
            config: std::path::PathBuf::new(),
            weather: BTreeMap::new(),
            weather_observations: None,
        }
    }

    fn policy() -> Operations {
        super::super::parse_operations(include_str!("../../../../traffic/operations.toml")).unwrap()
    }

    #[test]
    fn advisory_traffic_fits_inside_the_scene_cap() {
        assert_eq!(advisory_capacity(0), 2);
        assert_eq!(advisory_capacity(61), 2);
        assert_eq!(advisory_capacity(62), 1);
        assert_eq!(advisory_capacity(63), 0);
        assert_eq!(advisory_capacity(70), 0);
    }

    #[test]
    fn only_untowered_fields_near_the_player_are_selected() {
        let input = preparation(vec![
            station("KCTF", "CTAF", 122_900, 35.1),
            station("KTOW", "Tower", 118_300, 35.1),
            station("KTOW", "CTAF", 122_800, 35.1),
            station("KFAR", "CTAF", 122_700, 70.0),
        ]);
        assert_eq!(fields(&input, &policy()), vec!["KCTF".to_owned()]);
    }

    #[test]
    fn advisory_traffic_is_admitted_and_flown_without_a_controller() {
        let mut airport = openatc_core::airport::demo_airport();
        airport.icao = "KCTF".into();
        let input = preparation(vec![station("KCTF", "CTAF", 122_900, 35.0)]);
        let (plan, frame, kind) = plan(&input, &airport, 7, 0x0e00_0001).unwrap();
        assert_eq!(frame.target.id, 0x0e00_0001);
        assert_eq!(frame.target.com1_khz, Some(122_900));
        assert!(!frame.target.on_ground);
        assert!(!frame.target.callsign.is_empty());
        assert!(kind.is_some(), "advisory traffic keeps its propeller animation");

        let mut runtime = Runtime::default();
        let generation = runtime.world.generation();
        assert!(runtime
            .world
            .insert_advisory(generation, plan.id, plan.circuit.route(), 0));
        runtime.advisories.insert(plan.id, plan);
        // The published procedure is the authority, so the aircraft moves on its own.
        advance(&mut runtime, &BTreeSet::new());
        runtime.world.advance(0.0, false);
        runtime.world.advance(30.0, false);
        let pose = runtime
            .world
            .snapshot()
            .into_iter()
            .find(|pose| pose.id == 0x0e00_0001)
            .unwrap();
        assert!(pose.knots > 0.0);
        assert_eq!(pose.stage, openatc_core::traffic::motion::Stage::Descent);
        assert!(!pose.awaiting_clearance);
    }

    #[test]
    fn an_occupied_runway_is_never_landed_on() {
        let mut airport = openatc_core::airport::demo_airport();
        airport.icao = "KCTF".into();
        let input = preparation(vec![station("KCTF", "CTAF", 122_900, 35.0)]);
        let (plan, _, _) = plan(&input, &airport, 7, 0x0e00_0003).unwrap();
        let mut runtime = Runtime::default();
        let generation = runtime.world.generation();
        assert!(runtime
            .world
            .insert_advisory(generation, plan.id, plan.circuit.route(), 0));
        runtime.advisories.insert(plan.id, plan.clone());
        // Fly to the final approach.
        let mut clock = 0.0;
        for _ in 0..600 {
            let pose = runtime
                .world
                .snapshot()
                .into_iter()
                .find(|pose| pose.id == plan.id)
                .unwrap();
            if pose.segment == 2 {
                break;
            }
            if pose.awaiting_clearance {
                assert!(runtime
                    .world
                    .authorise_advisory(generation, plan.id, pose.segment, &BTreeSet::new()));
            }
            clock += 1.0;
            runtime.world.advance(clock, false);
        }
        let pose = runtime
            .world
            .snapshot()
            .into_iter()
            .find(|pose| pose.id == plan.id)
            .unwrap();
        assert_eq!(pose.segment, 2, "the aircraft is established on the final approach");
        // An occupied runway turns the aircraft away instead of landing.
        let occupied = BTreeSet::from([plan.pattern.runway_resource().to_owned()]);
        advance(&mut runtime, &occupied);
        let turned = runtime
            .world
            .snapshot()
            .into_iter()
            .find(|pose| pose.id == plan.id)
            .unwrap();
        assert_eq!(turned.stage, openatc_core::traffic::motion::Stage::Climb);
        let published = runtime.advisories.get(&plan.id).unwrap();
        assert!(published.go_around.is_some());
        assert!(published
            .go_around
            .as_ref()
            .is_some_and(|advisory| advisory.text.contains("going around")));
    }
    #[test]
    fn a_finished_advisory_circuit_is_continued_from_the_same_point() {
        let mut airport = openatc_core::airport::demo_airport();
        airport.icao = "KCTF".into();
        let input = preparation(vec![station("KCTF", "CTAF", 122_900, 35.0)]);
        let (plan, _, _) = plan(&input, &airport, 7, 0x0e00_0002).unwrap();
        let mut runtime = Runtime::default();
        let generation = runtime.world.generation();
        assert!(runtime
            .world
            .insert_advisory(generation, plan.id, plan.circuit.route(), 0));
        runtime.advisories.insert(plan.id, plan.clone());
        // Fly the whole circuit, granting each leg as the procedure reaches it.
        for step in 1..=600 {
            advance(&mut runtime, &BTreeSet::new());
            runtime.world.advance(f64::from(step), false);
            let pose = runtime
                .world
                .snapshot()
                .into_iter()
                .find(|pose| pose.id == plan.id)
                .unwrap();
            if pose.complete {
                break;
            }
        }
        let pose = runtime
            .world
            .snapshot()
            .into_iter()
            .find(|pose| pose.id == plan.id)
            .unwrap();
        assert!(pose.complete, "the circuit ends on the landing roll");
        // The next circuit starts where the landing roll ended.
        advance(&mut runtime, &BTreeSet::new());
        let continued = runtime
            .world
            .snapshot()
            .into_iter()
            .find(|pose| pose.id == plan.id)
            .unwrap();
        assert!(!continued.complete);
        assert_eq!(continued.segment, 0);
        assert_eq!(
            continued.stage,
            openatc_core::traffic::motion::Stage::Climb
        );
        let expected = plan.pattern.next_circuit().unwrap().route().start_position();
        assert!((continued.position.latitude - expected.latitude).abs() < 1e-6);
        assert!((continued.position.longitude - expected.longitude).abs() < 1e-6);
    }
}
