//! Build controller contracts from the checked flight, published stations and retained IAS inputs.
use super::{flight::Clearance, *};
use openatc_core::{
    dialogue::say,
    regions::Region,
    state::{State, TaxiClearance, Telemetry},
    stations::Station,
    traffic::{
        motion::{Route, Segment, Stage, Waypoint},
        trajectory::Fix,
    },
};

#[derive(Clone)]
pub(super) struct Flight {
    pub origin: Arc<Airport>,
    pub destination: Arc<Airport>,
    pub departure_runway: String,
    pub arrival_runway: String,
    pub terminal: Arc<routing::Terminal>,
    pub callsign: String,
    pub stations: Arc<[Station]>,
    pub regions: Arc<BTreeMap<String, Region>>,
    pub departure_weather: environment::Environment,
    pub arrival_weather: environment::Environment,
    pub missed: Option<super::flight::MissedInstruction>,
    pub scope_seconds: f64,
}
impl Flight {
    pub fn context(&self) -> Context<'_> {
        Context {
            origin: &self.origin,
            destination: &self.destination,
            departure_runway: &self.departure_runway,
            arrival_runway: &self.arrival_runway,
            terminal: &self.terminal,
            callsign: &self.callsign,
            stations: &self.stations,
            regions: &self.regions,
            departure_weather: &self.departure_weather,
            arrival_weather: &self.arrival_weather,
            missed: self.missed.as_ref(),
            scope_seconds: self.scope_seconds,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Context<'a> {
    pub origin: &'a Airport,
    pub destination: &'a Airport,
    pub departure_runway: &'a str,
    pub arrival_runway: &'a str,
    pub terminal: &'a routing::Terminal,
    pub callsign: &'a str,
    pub stations: &'a [Station],
    pub regions: &'a BTreeMap<String, Region>,
    pub departure_weather: &'a environment::Environment,
    pub arrival_weather: &'a environment::Environment,
    pub missed: Option<&'a super::flight::MissedInstruction>,
    pub scope_seconds: f64,
}

fn role(station: &Station, service: &str) -> bool {
    station.service == service || station.services.iter().any(|value| value == service)
}

pub(super) fn covers_recovery(station: &Station, route: &Route) -> bool {
    (0..512)
        .map_while(|index| route.segment(index))
        .all(|segment| {
            segment.points.iter().all(|point| {
                let observed = telemetry(*point, false);
                if role(station, "Center") && !station.coverage.is_empty() {
                    station
                        .coverage
                        .iter()
                        .any(|volume| volume.contains(&observed))
                } else {
                    openatc_core::stations::reception_distance(station, &observed).is_some()
                }
            })
        })
}

/// Keep a hold on its controlling frequency while coordinating any neighboring sector it enters.
pub(super) fn recovery_coordination(
    supervisor: &Station,
    route: &Route,
    stations: &[Station],
) -> Result<Vec<Station>, String> {
    if !role(supervisor, "Center") {
        return Err("Terminal recovery leaves its assigned controller coverage".into());
    }
    let mut participants = Vec::<Station>::new();
    for index in 0..512 {
        let Some(segment) = route.segment(index) else {
            break;
        };
        if segment.points.iter().any(|point| {
            openatc_core::stations::reception_distance(supervisor, &telemetry(*point, false))
                .is_none()
        }) {
            return Err("Coordinated holding route leaves its supervisor radio range".into());
        }
        let (first, transfers) = coordinated_centers(stations, segment)?;
        for station in
            std::iter::once(first).chain(transfers.into_iter().map(|transfer| transfer.station))
        {
            if !participants.iter().any(|existing| {
                existing.name == station.name
                    && existing.airport == station.airport
                    && existing.khz == station.khz
            }) {
                participants.push(station);
            }
        }
        if participants.len() > 32 {
            return Err("Recovery coordination exceeds its controller budget".into());
        }
    }
    Ok(participants)
}

fn telemetry(point: Waypoint, on_ground: bool) -> Telemetry {
    Telemetry {
        latitude: point.latitude,
        longitude: point.longitude,
        altitude_feet: point.altitude_feet,
        position_valid: true,
        on_ground,
        ..Default::default()
    }
}

/// The same great-circle interpolation used by the movement sampler.
struct SectorEdge {
    first: Waypoint,
    last: Waypoint,
    distance: f64,
    course: f64,
    angle: f64,
    z: f64,
    tangent_z: f64,
    longitude_momentum: f64,
}
impl SectorEdge {
    fn new(first: Waypoint, last: Waypoint) -> Result<Self, String> {
        if [
            first.latitude,
            first.longitude,
            first.altitude_feet,
            last.latitude,
            last.longitude,
            last.altitude_feet,
        ]
        .iter()
        .any(|value| !value.is_finite())
            || first.latitude.abs() >= 85.0
            || last.latitude.abs() >= 85.0
        {
            return Err("Invalid controller movement coordinates".into());
        }
        let distance = openatc_core::arrival::distance_nm(
            [first.latitude, first.longitude],
            [last.latitude, last.longitude],
        );
        let course = openatc_core::arrival::bearing(
            [first.latitude, first.longitude],
            [last.latitude, last.longitude],
        );
        let latitude = first.latitude.to_radians();
        let heading = course.to_radians();
        Ok(Self {
            first,
            last,
            distance,
            course,
            angle: distance / 3440.065,
            z: latitude.sin(),
            tangent_z: latitude.cos() * heading.cos(),
            longitude_momentum: latitude.cos() * heading.sin(),
        })
    }
    fn at(&self, fraction: f64) -> Waypoint {
        let coordinate = openatc_core::arrival::destination(
            [self.first.latitude, self.first.longitude],
            self.course,
            self.distance * fraction,
        );
        Waypoint {
            latitude: coordinate[0],
            longitude: coordinate[1],
            altitude_feet: self.first.altitude_feet
                + (self.last.altitude_feet - self.first.altitude_feet) * fraction,
        }
    }
    fn longitude_breaks(&self, origin: f64) -> Vec<f64> {
        if self.angle < 1e-12 {
            return Vec::new();
        }
        let latitude = self.first.latitude.to_radians();
        let longitude = self.first.longitude.to_radians();
        let heading = self.course.to_radians();
        let target = (origin + 180.0).to_radians();
        let first = [
            latitude.cos() * longitude.cos(),
            latitude.cos() * longitude.sin(),
        ];
        let tangent = [
            -heading.sin() * longitude.sin() - heading.cos() * latitude.sin() * longitude.cos(),
            heading.sin() * longitude.cos() - heading.cos() * latitude.sin() * longitude.sin(),
        ];
        let a = first[0] * target.sin() - first[1] * target.cos();
        let b = tangent[0] * target.sin() - tangent[1] * target.cos();
        let root = (-a).atan2(b);
        (-1..=1)
            .filter_map(|turn| {
                let along = root + f64::from(turn) * std::f64::consts::PI;
                let x = first[0] * along.cos() + tangent[0] * along.sin();
                let y = first[1] * along.cos() + tangent[1] * along.sin();
                (along > 0.0 && along < self.angle && x * target.cos() + y * target.sin() > 0.0)
                    .then_some(along / self.angle)
            })
            .collect()
    }
    /// A geographic polygon edge is linear in longitude/latitude. Its signed side along
    /// a great circle can turn twice; include both extrema before isolating crossings.
    fn extrema(&self, longitude_weight: f64, latitude_weight: f64) -> Vec<f64> {
        let mut points = vec![0.0, 1.0];
        if self.angle < 1e-12 || latitude_weight.abs() < 1e-14 {
            return points;
        }
        let amplitude2 = (self.z * self.z + self.tangent_z * self.tangent_z).min(1.0);
        if amplitude2 < 1e-20 {
            return points;
        }
        let phase = self.tangent_z.atan2(self.z);
        for turn in -1..=1 {
            let along = phase + f64::from(turn) * std::f64::consts::PI;
            if along > 0.0 && along < self.angle {
                points.push(along / self.angle);
            }
        }
        let ratio = longitude_weight * self.longitude_momentum / latitude_weight;
        let discriminant = ((amplitude2 - 1.0).powi(2) + 4.0 * ratio * ratio).sqrt();
        // Use the product of the roots for the smaller root to avoid cancellation.
        let large = (amplitude2 + 1.0 + discriminant) / 2.0;
        let small = (amplitude2 - ratio * ratio) / large;
        for square in [small, large] {
            if square < -1e-12 || square > amplitude2 + 1e-12 {
                continue;
            }
            for z in [square.max(0.0).sqrt(), -square.max(0.0).sqrt()] {
                let offset = (z / amplitude2.sqrt()).clamp(-1.0, 1.0).acos();
                for sign in [-1.0, 1.0] {
                    for turn in -1..=1 {
                        let along = phase + sign * offset + f64::from(turn) * std::f64::consts::TAU;
                        if along > 0.0 && along < self.angle {
                            points.push(along / self.angle);
                        }
                    }
                }
            }
        }
        points.sort_by(f64::total_cmp);
        points.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        points
    }
}

/// Partition the actual movement arc at every published sector boundary and level.
fn sector_cuts(edge: &SectorEdge, stations: &[&Station]) -> Result<Vec<f64>, String> {
    let mut cuts = vec![0.0, 1.0];
    for station in stations {
        for volume in &station.coverage {
            let height = edge.last.altitude_feet - edge.first.altitude_feet;
            if height.abs() > 1e-9 {
                for level in [volume.floor, volume.ceiling] {
                    let fraction = (level - edge.first.altitude_feet) / height;
                    if (0.0..1.0).contains(&fraction) {
                        cuts.push(fraction);
                    }
                }
            }
            let Some(origin) = volume.points.first() else {
                continue;
            };
            let longitude = |value: f64| (value - origin[1] + 180.0).rem_euclid(360.0) - 180.0;
            let breaks = edge.longitude_breaks(origin[1]);
            cuts.extend(breaks.iter().copied());
            for (a, b) in volume
                .points
                .iter()
                .zip(volume.points.iter().cycle().skip(1))
            {
                let start = [longitude(a[1]), a[0]];
                let delta = [longitude(b[1]) - start[0], b[0] - a[0]];
                let length2 = delta[0] * delta[0] + delta[1] * delta[1];
                if length2 < 1e-24 {
                    continue;
                }
                let side = |fraction| {
                    let point = edge.at(fraction);
                    (longitude(point.longitude) - start[0]) * delta[1]
                        - (point.latitude - start[1]) * delta[0]
                };
                let mut critical = edge.extrema(delta[1], -delta[0]);
                critical.extend(breaks.iter().copied());
                critical.sort_by(f64::total_cmp);
                critical.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
                for interval in critical.windows(2) {
                    let (mut low, mut high) = (interval[0], interval[1]);
                    // Evaluate inside each continuous longitude branch, below the
                    // polygon edge's roundoff tolerance even on a long movement arc.
                    let inset = ((1e-7 / (edge.distance * 1852.0).max(1.0))
                        .min((high - low) * 0.01))
                    .max(f64::EPSILON);
                    low += inset;
                    high -= inset;
                    if low >= high {
                        continue;
                    }
                    let mut left = side(low);
                    let right = side(high);
                    if left.abs() < 1e-14 {
                        high = low;
                    } else if right.abs() < 1e-14 {
                        low = high;
                    } else if left.signum() == right.signum() {
                        continue;
                    } else {
                        for _ in 0..48 {
                            let middle = low.midpoint(high);
                            let value = side(middle);
                            if value.signum() == left.signum() {
                                low = middle;
                                left = value;
                            } else {
                                high = middle;
                            }
                        }
                    }
                    let fraction = low.midpoint(high);
                    let point = edge.at(fraction);
                    let along = ((longitude(point.longitude) - start[0]) * delta[0]
                        + (point.latitude - start[1]) * delta[1])
                        / length2;
                    if (-1e-10..=1.0 + 1e-10).contains(&along) && (0.0..1.0).contains(&fraction) {
                        cuts.push(fraction);
                    }
                }
                if cuts.len() > 8192 {
                    return Err("Controller boundary partition budget exceeded".into());
                }
            }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    Ok(cuts)
}

fn coordinated_centers(
    stations: &[Station],
    segment: &Segment,
) -> Result<(Station, Vec<super::flight::SectorTransfer>), String> {
    if stations.len() > 65536
        || segment.points.len() < 2
        || !segment.knots.is_finite()
        || segment.knots <= 0.0
    {
        return Err("Invalid bounded controller planning input".into());
    }
    let mut centers = stations
        .iter()
        .filter(|station| {
            role(station, "Center")
                && (118000..=136990).contains(&station.khz)
                && !station.coverage.is_empty()
        })
        .collect::<Vec<_>>();
    centers.sort_by(|a, b| {
        a.airport
            .cmp(&b.airport)
            .then(a.name.cmp(&b.name))
            .then(a.khz.cmp(&b.khz))
    });
    // Alternate frequencies for one controller do not add new airspace partitions.
    centers.dedup_by(|a, b| a.airport == b.airport && a.name == b.name && a.coverage == b.coverage);
    let origin = segment.points[0].longitude;
    let longitude = |value: f64| (value - origin + 180.0).rem_euclid(360.0) - 180.0;
    let mut bounds = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for point in &segment.points {
        bounds[0] = bounds[0].min(point.latitude);
        bounds[1] = bounds[1].max(point.latitude);
        bounds[2] = bounds[2].min(longitude(point.longitude));
        bounds[3] = bounds[3].max(longitude(point.longitude));
        bounds[4] = bounds[4].min(point.altitude_feet);
        bounds[5] = bounds[5].max(point.altitude_feet);
    }
    centers.retain(|station| {
        station.coverage.iter().any(|volume| {
            if volume.floor > bounds[5] || volume.ceiling < bounds[4] {
                return false;
            }
            let mut area = [
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ];
            for point in &volume.points {
                area[0] = area[0].min(point[0]);
                area[1] = area[1].max(point[0]);
                area[2] = area[2].min(longitude(point[1]));
                area[3] = area[3].max(longitude(point[1]));
            }
            // Padding is only a search envelope; actual coverage is never widened.
            area[0] <= bounds[1] + 0.02
                && area[1] >= bounds[0] - 0.02
                && area[2] <= bounds[3] + 0.02
                && area[3] >= bounds[2] - 0.02
        })
    });
    if centers.is_empty() || centers.len() > 4096 {
        return Err("No bounded published enroute controller set".into());
    }
    let mut selected: Vec<(f64, Station)> = Vec::new();
    let mut elapsed = 0.0;
    let mut partitions = 0;
    for pair in segment.points.windows(2) {
        let edge = SectorEdge::new(pair[0], pair[1])?;
        let duration = edge.distance / segment.knots * 3600.0;
        let cuts = sector_cuts(&edge, &centers)?;
        for interval in cuts.windows(2) {
            partitions += 1;
            if partitions > 65536 {
                return Err("Controller route partition budget exceeded".into());
            }
            let fraction = interval[0].midpoint(interval[1]);
            let observed = telemetry(edge.at(fraction), false);
            let covers = |station: &Station| {
                station
                    .coverage
                    .iter()
                    .any(|volume| volume.contains(&observed))
            };
            if selected.last().is_some_and(|(_, station)| covers(station)) {
                continue;
            }
            let station = centers
                .iter()
                .find(|station| covers(station))
                .ok_or_else(|| {
                    format!(
                        "Published controller coverage has a gap at {:.9}/{:.9} {:.1} feet",
                        observed.latitude, observed.longitude, observed.altitude_feet
                    )
                })?;
            selected.push((elapsed + duration * interval[0], (*station).clone()));
            if selected.len() > 32 {
                return Err("Too many controller transfers within one movement".into());
            }
        }
        elapsed += duration;
    }
    let initial = selected
        .first()
        .ok_or("Empty enroute controller path")?
        .1
        .clone();
    let transfers = selected
        .into_iter()
        .skip(1)
        .map(|(seconds, station)| super::flight::SectorTransfer {
            remaining_seconds: (elapsed - seconds).max(0.0),
            station,
        })
        .collect();
    Ok((initial, transfers))
}

fn station(
    context: &Context<'_>,
    segment: &Segment,
    airport: &Airport,
    services: &[&str],
) -> Result<Station, String> {
    let mut candidates = context
        .stations
        .iter()
        .filter(|station| (118000..=136990).contains(&station.khz))
        .filter_map(|station| {
            services
                .iter()
                .position(|service| role(station, service))
                .map(|priority| (priority, station))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|(a, x), (b, y)| a.cmp(b).then(x.khz.cmp(&y.khz)).then(x.name.cmp(&y.name)));
    for (priority, candidate) in candidates {
        let center = services[priority] == "Center";
        if !center && candidate.airport != airport.icao {
            continue;
        }
        if segment.points.iter().all(|point| {
            let observed = telemetry(*point, segment.stage.on_ground());
            if center {
                candidate
                    .coverage
                    .iter()
                    .any(|volume| volume.contains(&observed))
            } else {
                openatc_core::stations::reception_distance(candidate, &observed).is_some()
            }
        }) {
            if !center || coordinated_centers(std::slice::from_ref(candidate), segment).is_ok() {
                return Ok(candidate.clone());
            }
        }
    }
    let closest = context
        .stations
        .iter()
        .filter(|candidate| services.iter().any(|service| role(candidate, service)))
        .map(|candidate| {
            (
                segment
                    .points
                    .iter()
                    .filter(|point| {
                        candidate
                            .coverage
                            .iter()
                            .any(|volume| volume.contains(&telemetry(**point, false)))
                    })
                    .count(),
                candidate.name.as_str(),
            )
        })
        .max_by_key(|(covered, _)| *covered);
    Err(format!(
        "No published controller covers {:?} at {} (from {:?} to {:?}; best coverage {:?} of {} samples)",
        segment.stage,
        airport.icao,
        segment.points.first(),
        segment.points.last(),
        closest,
        segment.points.len()
    ))
}

fn level(
    fix: &Fix,
    point: Waypoint,
    context: &Context<'_>,
    airport: &Airport,
) -> Result<String, String> {
    let atmosphere = fix
        .airspeed
        .ok_or("Controller contract is missing its indicated airspeed reference")?
        .atmosphere;
    let region = openatc_core::regions::region_for(context.regions, &airport.icao);
    let altitude = atmosphere.reported(point.altitude_feet, false)?;
    if altitude >= f64::from(region.transition_feet) {
        let standard = atmosphere.reported(point.altitude_feet, true)?;
        if (standard - (standard / 100.0).round() * 100.0).abs() > 1.0 {
            return Err(format!(
                "Generated crossing at {:.6}/{:.6} needs a cleared flight-level solution ({standard:.2} standard-pressure feet)",
                point.latitude, point.longitude
            ));
        }
        Ok(say(
            "generated_contract_flight_level",
            &[("level", format!("{:.0}", standard / 100.0))],
        ))
    } else {
        Ok(openatc_core::altitude_text(
            altitude.round(),
            openatc_core::regions::units_for_region(&region),
            true,
        ))
    }
}

/// Choose a standard flight level inside the solved physical envelope above transition altitude.
pub(super) fn select_level(
    fix: &Fix,
    region: &Region,
    lower: f64,
    upper: f64,
    preferred: f64,
) -> Option<f64> {
    let atmosphere = fix.airspeed?.atmosphere;
    let preferred = preferred.clamp(lower, upper);
    let transition = atmosphere
        .geometric(f64::from(region.transition_feet), false)
        .ok()?;
    if preferred < transition {
        return Some(preferred);
    }
    let low = (atmosphere.reported(lower, true).ok()? / 100.0 - 1e-8).ceil();
    let high = (atmosphere.reported(upper, true).ok()? / 100.0 + 1e-8).floor();
    if low <= high {
        let level = (atmosphere.reported(preferred, true).ok()? / 100.0)
            .round()
            .clamp(low, high);
        let selected = atmosphere.geometric(level * 100.0, true).ok()?;
        if (lower - 1e-5..=upper + 1e-5).contains(&selected) {
            return Some(selected.clamp(lower, upper));
        }
    }
    let below = atmosphere
        .geometric(f64::from(region.transition_feet) - 1.0, false)
        .ok()?
        .min(upper);
    (lower <= below).then(|| preferred.clamp(lower, below))
}

fn contract(station: Station, instruction: String, readback: String) -> Clearance {
    Clearance {
        station,
        transfers: Vec::new(),
        coordinated_recovery: Vec::new(),
        instruction,
        readback,
        initial_climb: None,
        taxi_through: None,
        airborne_through: None,
        landing_rollout: None,
        missed: None,
        holding: None,
    }
}

/// Add the same contingent limit to every movement covered by a single clearance.
pub(super) fn holding_contracts(
    route: &Route,
    requirements: &[Fix],
    holds: &BTreeMap<usize, super::recovery::LimitHold>,
    context: &Context<'_>,
    contracts: &mut BTreeMap<usize, Clearance>,
) -> Result<(), String> {
    let mut scope: BTreeMap<usize, usize> = holds.keys().map(|index| (*index, *index)).collect();
    for (&start, clearance) in contracts.iter() {
        if let Some(end) = clearance.airborne_through {
            if holds.contains_key(&end) {
                for index in start..=end {
                    scope.insert(index, end);
                }
            }
        }
    }
    if let Some(end) = contracts.get(&1).and_then(|c| c.initial_climb) {
        if !holds.contains_key(&end) {
            return Err("Initial SID has no contingent holding limit".into());
        }
        scope.insert(1, end);
        for index in 2..=end {
            scope.insert(index, end);
        }
    }
    for (index, boundary) in scope {
        let hold = holds
            .get(&boundary)
            .ok_or("Contingent holding scope is incomplete")?;
        let segment = route
            .segment(boundary)
            .ok_or("Holding boundary is missing")?;
        let fix = requirements
            .get(boundary.checked_sub(2).ok_or("Holding index is invalid")?)
            .ok_or("Holding instruction has no pressure reference")?;
        let (airport, weather) = if segment.stage == Stage::Descent {
            (context.destination, context.arrival_weather)
        } else {
            (context.origin, context.departure_weather)
        };
        let course = (hold.inbound_true_degrees - weather.variation)
            .rem_euclid(360.0)
            .round() as i32;
        let slots = [
            (
                "latitude",
                format!(
                    "{:.5} degrees {}",
                    hold.position.latitude.abs(),
                    if hold.position.latitude < 0.0 {
                        "south"
                    } else {
                        "north"
                    }
                ),
            ),
            (
                "longitude",
                format!(
                    "{:.5} degrees {}",
                    hold.position.longitude.abs(),
                    if hold.position.longitude < 0.0 {
                        "west"
                    } else {
                        "east"
                    }
                ),
            ),
            (
                "course",
                format!(
                    "{:03}",
                    if course == 0 || course == 360 {
                        360
                    } else {
                        course
                    }
                ),
            ),
            ("altitude", level(fix, hold.position, context, airport)?),
            ("speed", format!("{:.0}", hold.indicated_knots)),
            ("seconds", format!("{:.0}", hold.leg_seconds)),
        ];
        let clearance = contracts
            .get_mut(&index)
            .ok_or("Holding instruction has no controller contract")?;
        clearance.instruction = format!(
            "{} {}",
            clearance.instruction,
            say("generated_limit_holding", &slots)
        );
        clearance.readback = format!(
            "{} {}",
            clearance.readback,
            say("generated_limit_holding_readback", &slots)
        );
        clearance.holding = Some(hold.clone());
    }
    Ok(())
}

pub(super) fn contracts(
    route: &Route,
    map: usize,
    requirements: &[Fix],
    surface_start: usize,
    orders: &[(usize, TaxiClearance)],
    context: &Context<'_>,
) -> Result<BTreeMap<usize, Clearance>, String> {
    if context.callsign.trim().is_empty()
        || context.callsign.len() > 32
        || context
            .callsign
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && c != '-')
        || !(60.0..=900.0).contains(&context.scope_seconds)
        || requirements.is_empty()
        || requirements.len() + 1 != map
        || context.departure_runway.is_empty()
        || context.arrival_runway.is_empty()
        || context.terminal.departure_name.is_empty()
        || context.terminal.arrival_name.is_empty()
        || context.terminal.approach_name.is_empty()
    {
        return Err(
            "Controller construction needs the actual complete flight identity and requirements"
                .into(),
        );
    }
    if !context.departure_weather.fresh()
        || !context.arrival_weather.fresh()
        || [
            (&requirements[0], context.departure_weather),
            (requirements.last().unwrap(), context.arrival_weather),
        ]
        .iter()
        .any(|(fix, weather)| {
            fix.airspeed.is_none_or(|speed| {
                speed.atmosphere != weather.atmosphere
                    || speed.wind_from != weather.wind_from
                    || speed.wind_knots != weather.wind_knots
            })
        })
    {
        return Err("Controller contracts need current weather matching the planned flight".into());
    }
    let missed_altitude = if let Some(instruction) = context.missed {
        if context
            .terminal
            .missed
            .last()
            .is_none_or(|leg| leg.terminator != "HM" || leg.identifier != instruction.fix)
            || !instruction.geometric_feet.is_finite()
            || (context
                .arrival_weather
                .atmosphere
                .reported(instruction.geometric_feet, instruction.standard)?
                - f64::from(instruction.reported_feet))
            .abs()
                > 1.0
        {
            return Err(
                "Missed amendment does not match its published fix or pressure reference".into(),
            );
        }
        if instruction.standard {
            if instruction.reported_feet % 100 != 0 {
                return Err("Missed amendment is not an assignable flight level".into());
            }
            Some(say(
                "generated_contract_flight_level",
                &[("level", (instruction.reported_feet / 100).to_string())],
            ))
        } else {
            let region =
                openatc_core::regions::region_for(context.regions, &context.destination.icao);
            Some(openatc_core::altitude_text(
                f64::from(instruction.reported_feet),
                openatc_core::regions::units_for_region(&region),
                true,
            ))
        }
    } else {
        None
    };
    let runway_key = |airport: &Airport, name: &str| {
        airport
            .runways
            .iter()
            .find(|runway| runway.first_name == name || runway.second_name == name)
            .map(|runway| {
                format!(
                    "RUNWAY/{}/{}/{}",
                    airport.icao, runway.first_name, runway.second_name
                )
            })
    };
    let departure_key = runway_key(context.origin, context.departure_runway)
        .ok_or("Departure runway is unknown")?;
    let arrival_key = runway_key(context.destination, context.arrival_runway)
        .ok_or("Arrival runway is unknown")?;
    if (0..=1).any(|index| {
        route
            .segment(index)
            .is_none_or(|segment| !segment.resources.contains(&departure_key))
    }) || route
        .segment(map + 2)
        .is_none_or(|segment| !segment.resources.contains(&arrival_key))
    {
        return Err("Controller runway names do not match the flight's protected resources".into());
    }
    let mut bounds = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for index in 0..512 {
        let Some(segment) = route.segment(index) else {
            break;
        };
        for point in &segment.points {
            bounds[0] = bounds[0].min(point.latitude);
            bounds[1] = bounds[1].max(point.latitude);
            bounds[2] = bounds[2].min(point.longitude);
            bounds[3] = bounds[3].max(point.longitude);
        }
    }
    let relevant = context
        .stations
        .iter()
        .filter(|station| {
            station.airport == context.origin.icao
                || station.airport == context.destination.icao
                || role(station, "Center")
                    && station.coverage.iter().any(|volume| {
                        let mut area = [
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                        ];
                        for point in &volume.points {
                            area[0] = area[0].min(point[0]);
                            area[1] = area[1].max(point[0]);
                            area[2] = area[2].min(point[1]);
                            area[3] = area[3].max(point[1]);
                        }
                        area[0] <= bounds[1]
                            && area[1] >= bounds[0]
                            && area[2] <= bounds[3]
                            && area[3] >= bounds[2]
                    })
        })
        .take(2049)
        .cloned()
        .collect::<Vec<_>>();
    if relevant.len() > 2048 {
        return Err("Controller corridor exceeds its station planning budget".into());
    }
    let selected_context = Context {
        stations: &relevant,
        ..*context
    };
    let context = &selected_context;
    let flare = map + 1;
    if route
        .segment(flare)
        .is_none_or(|segment| segment.stage != Stage::Approach)
        || route
            .segment(flare + 1)
            .is_none_or(|segment| segment.stage != Stage::Rollout)
    {
        return Err("Landing clearance has no final/rollout join".into());
    }
    let rollout_end = (flare + 1..512)
        .take_while(|index| {
            route
                .segment(*index)
                .is_some_and(|segment| segment.stage == Stage::Rollout)
        })
        .last()
        .unwrap();
    let initial_end = (2..=map)
        .take_while(|index| {
            route
                .segment(*index)
                .is_some_and(|segment| segment.stage == Stage::Climb)
                && route
                    .nominal_seconds(1, *index)
                    .is_some_and(|seconds| seconds <= context.scope_seconds)
        })
        .last()
        .ok_or("Initial SID has no published crossing within the bounded controller scope")?;
    let climb_geometry = Segment {
        stage: Stage::Climb,
        points: (2..=initial_end)
            .flat_map(|index| route.segment(index).unwrap().points.iter().copied())
            .collect(),
        knots: route.segment(2).unwrap().knots,
        resources: vec![],
        dwell_seconds: 0.0,
    };
    let climb_station = station(
        context,
        &climb_geometry,
        context.origin,
        &["Departure", "Approach", "Center"],
    )?;
    let initial_altitude = level(
        &requirements[initial_end - 2],
        *route.segment(initial_end).unwrap().points.last().unwrap(),
        context,
        context.origin,
    )?;
    let initial_slots = [
        ("callsign", context.callsign.into()),
        ("sid", context.terminal.departure_name.clone()),
        ("altitude", initial_altitude.clone()),
    ];
    let climb_contract = contract(
        climb_station,
        say("automatic_climb_via_sid", &initial_slots),
        say("generated_contract_sid_climb_readback", &initial_slots),
    );
    let mut result: BTreeMap<usize, Clearance> = BTreeMap::new();
    let mut approach_wording: Option<(String, String)> = None;
    let mut taxi_readbacks: BTreeMap<usize, String> = BTreeMap::new();
    for index in 0..512 {
        let Some(segment) = route.segment(index) else {
            break;
        };
        if segment.stage == Stage::Turnaround {
            continue;
        }
        let mut clearance = if index == 0 {
            let tower = station(context, segment, context.origin, &["Tower"])?;
            let slots = [
                ("callsign", context.callsign.into()),
                ("runway", context.departure_runway.into()),
            ];
            contract(
                tower,
                say("generated_contract_lineup", &slots),
                say("generated_contract_lineup_readback", &slots),
            )
        } else if index == 1 {
            let tower = station(context, segment, context.origin, &["Tower"])?;
            let slots = [
                ("callsign", context.callsign.into()),
                ("runway", context.departure_runway.into()),
                ("sid", context.terminal.departure_name.clone()),
                ("altitude", initial_altitude.clone()),
            ];
            let mut clearance = contract(
                tower,
                say("generated_contract_takeoff", &slots),
                say("generated_contract_takeoff_readback", &slots),
            );
            clearance.initial_climb = Some(initial_end);
            clearance
        } else if (2..=initial_end).contains(&index) {
            climb_contract.clone()
        } else if index == flare {
            let tower = station(context, segment, context.destination, &["Tower"])?;
            let slots = [
                ("callsign", context.callsign.into()),
                ("runway", context.arrival_runway.into()),
                ("weather", String::new()),
            ];
            let mut clearance = contract(
                tower,
                say("arrival_landing_clearance", &slots),
                say("pilot_landing_readback", &slots),
            );
            clearance.landing_rollout = Some(rollout_end);
            clearance
        } else if (flare + 1..=rollout_end).contains(&index) {
            let mut clearance = result
                .get(&flare)
                .cloned()
                .ok_or("Missing constructed landing contract")?;
            clearance.landing_rollout = None;
            clearance
        } else if index < flare {
            let fix = requirements
                .get(index - 2)
                .ok_or("Airborne contract has no crossing requirement")?;
            let (airport, services, route_name): (&Airport, &[&str], &str) = match segment.stage {
                Stage::Climb => (
                    context.origin,
                    &["Departure", "Approach", "Center"],
                    &context.terminal.departure_name,
                ),
                Stage::Cruise => (context.origin, &["Center"], ""),
                Stage::Descent => (
                    context.destination,
                    &["Approach", "Center"],
                    &context.terminal.arrival_name,
                ),
                Stage::Approach => (
                    context.destination,
                    &["Approach", "Tower"],
                    &context.terminal.approach_name,
                ),
                _ => return Err("Unsupported airborne controller phase".into()),
            };
            let (control, transfers) = match station(context, segment, airport, services) {
                Ok(station) => (station, Vec::new()),
                Err(error) if services.contains(&"Center") => {
                    coordinated_centers(context.stations, segment)
                        .map_err(|gap| format!("{error}: {gap}"))?
                }
                Err(error) => return Err(error),
            };
            let mut built = if segment.stage == Stage::Approach {
                let slots = [
                    ("callsign", context.callsign.into()),
                    ("approach", crate::flight::approach_label(route_name)),
                ];
                let (instruction, readback) = approach_wording
                    .get_or_insert_with(|| {
                        let instruction = say("arrival_approach_clearance", &slots);
                        if let (Some(missed), Some(altitude)) =
                            (context.missed, missed_altitude.as_ref())
                        {
                            let amendment = say(
                                "generated_missed_level_amendment",
                                &[("altitude", altitude.clone()), ("fix", missed.fix.clone())],
                            );
                            let reply = say(
                                "generated_approach_amended_readback",
                                &[
                                    ("callsign", context.callsign.into()),
                                    ("approach", crate::flight::approach_label(route_name)),
                                    ("altitude", altitude.clone()),
                                    ("fix", missed.fix.clone()),
                                ],
                            );
                            (format!("{instruction} {amendment}"), reply)
                        } else {
                            (instruction, say("pilot_approach_clearance", &slots))
                        }
                    })
                    .clone();
                let mut clearance = contract(control, instruction, readback);
                clearance.missed = context.missed.cloned();
                clearance
            } else {
                let first = segment.points[0];
                let last = *segment.points.last().unwrap();
                let (instruction, readback) = if last.altitude_feet > first.altitude_feet + 1.0 {
                    (
                        "generated_contract_climb",
                        "generated_contract_climb_readback",
                    )
                } else if last.altitude_feet < first.altitude_feet - 1.0 {
                    (
                        "generated_contract_descent",
                        "generated_contract_descent_readback",
                    )
                } else {
                    (
                        "generated_contract_level",
                        "generated_contract_level_readback",
                    )
                };
                let speed = fix
                    .airspeed
                    .ok_or("Airborne readback is missing IAS")?
                    .indicated_knots;
                let slots = [
                    ("callsign", context.callsign.into()),
                    (
                        "route",
                        if route_name.is_empty() {
                            say("generated_contract_filed_route", &[])
                        } else {
                            route_name.into()
                        },
                    ),
                    ("altitude", level(fix, last, context, airport)?),
                    ("speed", format!("{speed:.0}")),
                ];
                contract(control, say(instruction, &slots), say(readback, &slots))
            };
            built.transfers = transfers;
            built
        } else {
            let local = index
                .checked_sub(surface_start)
                .ok_or("Arrival controller has an unknown surface join")?;
            let (order_start, order) = orders
                .iter()
                .rev()
                .find(|(start, _)| *start <= local)
                .ok_or("Arrival taxi movement lacks its actual instruction")?;
            if order.airport != context.destination.icao
                || segment.stage == Stage::Crossing && order.crossing_runway.is_empty()
                || segment.stage != Stage::Crossing && !order.crossing_runway.is_empty()
            {
                return Err("Arrival taxi contract crosses a separate runway permission".into());
            }
            let ground = station(context, segment, context.destination, &["Ground", "Tower"])?;
            let mut actor = State::default();
            actor.plan.callsign = context.callsign.into();
            actor.taxi_clearance = order.clone();
            actor.taxi_clearance.pending_readback = true;
            openatc_core::apply::add_transmission(
                &mut actor,
                "ATC",
                &order.instructions,
                &openatc_core::state::SpeechTag {
                    position: ground.name.clone(),
                    ..Default::default()
                },
            );
            let readback = if let Some(existing) = taxi_readbacks.get(order_start) {
                existing.clone()
            } else {
                let reply = openatc_core::readback::auto_reply(&actor)
                    .ok_or("No complete taxi readback can be constructed")?;
                taxi_readbacks.insert(*order_start, reply.text.clone());
                reply.text
            };
            contract(
                ground,
                say(
                    "generated_surface_instruction",
                    &[
                        ("callsign", context.callsign.into()),
                        ("instructions", order.instructions.clone()),
                    ],
                ),
                readback,
            )
        };
        clearance.instruction = clearance.instruction.trim().into();
        result.insert(index, clearance);
    }
    // Group only identical instructions within one phase; the landing permission is a separate scope.
    let mut start = 2;
    while start < 512 {
        let Some(first) = route.segment(start) else {
            break;
        };
        if start <= initial_end {
            start = initial_end + 1;
            continue;
        }
        if !matches!(
            first.stage,
            Stage::Climb | Stage::Cruise | Stage::Descent | Stage::Approach | Stage::TaxiIn
        ) || start == 2
            || start == flare
        {
            start += 1;
            continue;
        }
        let Some(initial) = result
            .get(&start)
            .filter(|contract| contract.transfers.is_empty())
        else {
            start += 1;
            continue;
        };
        let end = (start + 1..512)
            .take_while(|index| {
                route
                    .segment(*index)
                    .is_some_and(|next| next.stage == first.stage)
                    && route
                        .nominal_seconds(start, *index)
                        .is_some_and(|seconds| seconds <= context.scope_seconds)
                    && result.get(index).is_some_and(|next| {
                        next.landing_rollout.is_none()
                            && next.transfers.is_empty()
                            && next.instruction == initial.instruction
                            && next.readback == initial.readback
                            && next.missed == initial.missed
                            && next.station.khz == initial.station.khz
                            && next.station.airport == initial.station.airport
                            && next.station.name == initial.station.name
                            && next.station.service == initial.station.service
                    })
                    && !orders
                        .iter()
                        .any(|(local, _)| surface_start + local == *index)
            })
            .last()
            .unwrap_or(start);
        if end > start {
            let initial = result.get_mut(&start).unwrap();
            if first.stage == Stage::TaxiIn {
                initial.taxi_through = Some(end);
            } else {
                initial.airborne_through = Some(end);
            }
        }
        start = end + 1;
    }
    if !context.departure_weather.fresh() || !context.arrival_weather.fresh() {
        return Err("Planning weather expired during controller construction".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinated_sectors_keep_real_receivers_and_reject_horizontal_and_vertical_gaps() {
        let center = |name: &str, west: f64, east: f64, floor: f64, ceiling: f64| Station {
            name: name.into(),
            service: "Center".into(),
            khz: 118675,
            coverage: vec![openatc_core::stations::Coverage {
                floor,
                ceiling,
                points: vec![[-1.0, west], [-1.0, east], [1.0, east], [1.0, west]],
            }],
            ..Default::default()
        };
        let segment = Segment {
            stage: Stage::Cruise,
            points: vec![
                Waypoint {
                    latitude: 0.0,
                    longitude: -0.1,
                    altitude_feet: 10000.0,
                },
                Waypoint {
                    latitude: 0.0,
                    longitude: 0.1,
                    altitude_feet: 10000.0,
                },
            ],
            knots: 240.0,
            dwell_seconds: 0.0,
            resources: vec![],
        };
        let west = center("West", -1.0, 0.0, 0.0, 45000.0);
        let east = center("East", 0.0, 1.0, 0.0, 45000.0);
        let (issuer, transfers) =
            coordinated_centers(&[west.clone(), east.clone()], &segment).unwrap();
        assert_eq!(issuer.name, "West");
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].station.name, "East");
        assert_eq!(
            transfers[0].station.khz, issuer.khz,
            "Same-frequency sectors still have distinct controller ownership"
        );
        assert!((transfers[0].remaining_seconds - 90.06).abs() < 0.1);
        let gap = center("East", 0.000001, 1.0, 0.0, 45000.0);
        assert!(
            coordinated_centers(&[west, gap], &segment).is_err(),
            "A gap between route samples cannot be bridged silently"
        );
        let mut climbing = segment.clone();
        climbing.points[0].altitude_feet = 9000.0;
        climbing.points[1].altitude_feet = 11000.0;
        let lower = center("Lower", -1.0, 1.0, 0.0, 10000.0);
        let upper = center("Upper", -1.0, 1.0, 10000.0, 45000.0);
        let (_, transfers) = coordinated_centers(&[lower.clone(), upper], &climbing).unwrap();
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].station.name, "Upper");
        let upper = center("Upper", -1.0, 1.0, 10000.01, 45000.0);
        assert!(coordinated_centers(&[lower, upper], &climbing).is_err());
        assert!(coordinated_centers(&[], &segment).is_err());
    }

    #[test]
    fn curved_movement_detects_a_sector_gap_even_when_both_endpoints_are_inside() {
        let segment = Segment {
            stage: Stage::Cruise,
            knots: 240.0,
            resources: vec![],
            dwell_seconds: 0.0,
            points: vec![
                Waypoint {
                    latitude: 45.0,
                    longitude: -0.1,
                    altitude_feet: 10000.0,
                },
                Waypoint {
                    latitude: 45.0,
                    longitude: 0.1,
                    altitude_feet: 10000.0,
                },
            ],
        };
        let lower = Station {
            name: "Lower".into(),
            service: "Center".into(),
            khz: 118030,
            coverage: vec![openatc_core::stations::Coverage {
                floor: 0.0,
                ceiling: 50000.0,
                points: vec![[44.0, -1.0], [44.0, 1.0], [45.00002, 1.0], [45.00002, -1.0]],
            }],
            ..Default::default()
        };
        assert!(
            segment
                .points
                .iter()
                .all(|point| lower.coverage[0].contains(&telemetry(*point, false)))
        );
        let edge = SectorEdge::new(segment.points[0], segment.points[1]).unwrap();
        assert!(edge.at(0.5).latitude > 45.00004);
        assert!(coordinated_centers(std::slice::from_ref(&lower), &segment).is_err());
        let mut upper = lower.clone();
        upper.name = "Upper".into();
        upper.coverage[0].points =
            vec![[45.00002, -1.0], [45.00002, 1.0], [46.0, 1.0], [46.0, -1.0]];
        let (_, transfers) =
            coordinated_centers(&[lower.clone(), upper.clone()], &segment).unwrap();
        assert_eq!(transfers.len(), 2);
        assert_eq!(transfers[0].station.name, "Upper");
        assert_eq!(transfers[1].station.name, "Lower");
        upper.coverage[0].points[0][0] = 45.00003;
        upper.coverage[0].points[1][0] = 45.00003;
        assert!(coordinated_centers(&[lower, upper], &segment).is_err());
    }

    #[test]
    fn sector_handoff_crosses_the_dateline_without_discovering_the_opposite_hemisphere() {
        let west = Station {
            name: "West".into(),
            service: "Center".into(),
            khz: 118030,
            coverage: vec![openatc_core::stations::Coverage {
                floor: 0.0,
                ceiling: 50000.0,
                points: vec![[10.0, 178.0], [10.0, 180.0], [12.0, 180.0], [12.0, 178.0]],
            }],
            ..Default::default()
        };
        let mut east = west.clone();
        east.name = "East".into();
        east.coverage[0].points =
            vec![[10.0, 180.0], [10.0, -178.0], [12.0, -178.0], [12.0, 180.0]];
        let mut unrelated = west.clone();
        unrelated.name = "Unrelated".into();
        unrelated.coverage[0].points = vec![[10.0, -1.0], [10.0, 1.0], [12.0, 1.0], [12.0, -1.0]];
        let movement = Segment {
            stage: Stage::Cruise,
            knots: 240.0,
            resources: vec![],
            dwell_seconds: 0.0,
            points: vec![
                Waypoint {
                    latitude: 11.0,
                    longitude: 179.8,
                    altitude_feet: 10000.0,
                },
                Waypoint {
                    latitude: 11.0,
                    longitude: -179.8,
                    altitude_feet: 10000.0,
                },
            ],
        };
        let (sender, transfers) =
            coordinated_centers(&[west.clone(), east.clone(), unrelated], &movement).unwrap();
        assert_eq!(sender.name, "West");
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].station.name, "East");
        east.coverage[0].points[0][1] = -179.999999;
        east.coverage[0].points[3][1] = -179.999999;
        assert!(coordinated_centers(&[west, east], &movement).is_err());
    }

    #[test]
    fn clearance_grid_uses_pressure_levels_and_never_relaxes_a_crossing() {
        let atmosphere = openatc_core::traffic::air::Atmosphere::surface(990.0, 25.0, 0.0).unwrap();
        let fix = Fix {
            coordinate: [45.0, 8.0],
            arrival_heading: None,
            minimum_feet: 0.0,
            maximum_feet: 40000.0,
            preferred_feet: 11000.0,
            ground_knots: 200.0,
            required_turn: None,
            intercept: None,
            airspeed: Some(openatc_core::traffic::trajectory::Airspeed {
                indicated_knots: 180.0,
                atmosphere,
                wind_from: 0.0,
                wind_knots: 0.0,
            }),
        };
        let mut region = openatc_core::regions::builtin_region("LIMC");
        region.transition_feet = 5000;
        let geometric = |level| atmosphere.geometric(level, true).unwrap();
        let chosen = select_level(
            &fix,
            &region,
            geometric(10500.0),
            geometric(11500.0),
            geometric(11049.0),
        )
        .unwrap();
        assert!((atmosphere.reported(chosen, true).unwrap() - 11000.0).abs() < 1e-6);
        assert!(
            select_level(
                &fix,
                &region,
                geometric(11010.0),
                geometric(11090.0),
                geometric(11050.0)
            )
            .is_none()
        );
        let low = atmosphere.geometric(3500.0, false).unwrap();
        assert_eq!(select_level(&fix, &region, low, low, low), Some(low));
        let missing = Fix {
            airspeed: None,
            ..fix
        };
        assert!(select_level(&missing, &region, 10000.0, 12000.0, 11000.0).is_none());
    }
}
