//! Route progress in simulator time. Clearances remain the controller's responsibility.
use crate::arrival::{bearing, destination, distance_nm};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A movement ends at each clearance limit, even if the next segment shares its runway.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Occupying the departure stand.
    Parked,
    /// Tug movement out of the stand.
    Pushback,
    /// Cleared surface movement towards departure.
    TaxiOut,
    /// One specifically authorised runway crossing.
    Crossing,
    /// Specifically cleared runway entry and alignment before the takeoff roll.
    LineUp,
    /// Takeoff roll, ending at rotation.
    Takeoff,
    /// Published or cleared climb path.
    Climb,
    /// Cleared en-route legs.
    Cruise,
    /// A terrain-checked closed holding pattern with an explicit onward clearance.
    Holding,
    /// Cleared descent path.
    Descent,
    /// Instrument approach before touchdown.
    Approach,
    /// Validated missed-approach route.
    MissedApproach,
    /// Landing roll and runway exit.
    Rollout,
    /// Cleared surface movement to parking.
    TaxiIn,
    /// Stand occupancy before the next flight.
    Turnaround,
}
impl Stage {
    /// Whether terrain contact is required when drawing this movement.
    #[must_use]
    pub fn on_ground(self) -> bool {
        matches!(
            self,
            Self::Parked
                | Self::Pushback
                | Self::TaxiOut
                | Self::Crossing
                | Self::LineUp
                | Self::Takeoff
                | Self::Rollout
                | Self::TaxiIn
                | Self::Turnaround
        )
    }
}

/// A resolved geographic point, independent of the simulator's local origin.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct Waypoint {
    /// Degrees north.
    pub latitude: f64,
    /// Degrees east.
    pub longitude: f64,
    /// Feet MSL, including terrain elevation for surface paths.
    pub altitude_feet: f64,
}
impl Waypoint {
    fn coordinate(self) -> [f64; 2] {
        [self.latitude, self.longitude]
    }
    fn valid(self) -> bool {
        (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
            && (-2000.0..=60000.0).contains(&self.altitude_feet)
    }
}

/// Coordinates and altitudes have already been resolved against scenery and navdata.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Segment {
    /// Operational movement represented by this segment.
    pub stage: Stage,
    /// Ordered continuous path; one point represents a stationary dwell.
    pub points: Vec<Waypoint>,
    /// Planned ground speed; terrain and procedure validation happens before compilation.
    pub knots: f64,
    /// Gate, taxi conflict or runway resources held until the entire segment is complete.
    /// A crossing segment must end beyond the far holding line, including tail clearance.
    pub resources: Vec<String>,
    /// Stationary preparation/turnaround time, in simulator seconds.
    pub dwell_seconds: f64,
}

#[derive(Clone, Debug)]
struct Leg {
    from: Waypoint,
    to: Waypoint,
    duration: f64,
    course: f64,
}

/// Compiled once on a worker; sampling never searches navdata or recomputes leg lengths.
#[derive(Clone, Debug)]
pub struct Route {
    segments: Vec<Segment>,
    legs: Vec<Vec<Leg>>,
}
impl Route {
    /// Reject discontinuities, oversized geometry and infeasible vertical profiles.
    pub fn compile(
        segments: Vec<Segment>,
        climb_fpm: f64,
        descent_fpm: f64,
    ) -> Result<Self, String> {
        if segments.is_empty()
            || segments.len() > 512
            || !(100.0..=10000.0).contains(&climb_fpm)
            || !(100.0..=10000.0).contains(&descent_fpm)
        {
            return Err("Invalid traffic route or vertical performance".into());
        }
        let mut legs = Vec::with_capacity(segments.len());
        let mut count = 0;
        let mut previous: Option<Waypoint> = None;
        for segment in &segments {
            count += segment.points.len();
            if segment.points.is_empty()
                || count > 4096
                || segment.resources.len() > 32
                || segment
                    .resources
                    .iter()
                    .any(|r| r.is_empty() || r.len() > 128)
                || !(0.0..=86400.0).contains(&segment.dwell_seconds)
                || !(0.0..=600.0).contains(&segment.knots)
                || segment.points.iter().any(|p| !p.valid())
            {
                return Err("Traffic segment exceeds its geometry or memory limits".into());
            }
            let first = segment.points[0];
            if previous.is_some_and(|p| {
                distance_nm(p.coordinate(), first.coordinate()) > 0.001
                    || (p.altitude_feet - first.altitude_feet).abs() > 1.0
            }) {
                return Err("Traffic route contains a discontinuity".into());
            }
            if matches!(
                segment.stage,
                Stage::Crossing | Stage::LineUp | Stage::Takeoff | Stage::Rollout
            ) && segment.resources.is_empty()
            {
                return Err("Runway movement has no protected resource".into());
            }
            let mut compiled = Vec::new();
            for pair in segment.points.windows(2) {
                let (from, to) = (pair[0], pair[1]);
                let nm = distance_nm(from.coordinate(), to.coordinate());
                if nm < 0.00001 || segment.knots < 0.1 {
                    return Err("Moving traffic needs a nonzero leg and speed".into());
                }
                let seconds = nm / segment.knots * 3600.0;
                let vertical = (to.altitude_feet - from.altitude_feet) * 60.0 / seconds;
                // Sampled interpolation can exceed an exact rate by floating-point roundoff.
                let tolerance = climb_fpm.max(descent_fpm) * 1e-9;
                if vertical > climb_fpm + tolerance || -vertical > descent_fpm + tolerance {
                    return Err("Traffic route exceeds climb or descent performance".into());
                }
                compiled.push(Leg {
                    from,
                    to,
                    duration: seconds,
                    course: bearing(from.coordinate(), to.coordinate()),
                });
            }
            previous = segment.points.last().copied();
            if segment.stage == Stage::Holding
                && (segment.points.len() < 4
                    || distance_nm(first.coordinate(), previous.unwrap().coordinate()) > 0.001
                    || segment
                        .points
                        .iter()
                        .any(|point| (point.altitude_feet - first.altitude_feet).abs() > 1.0)
                    || compiled.iter().map(|leg| leg.duration).sum::<f64>() < 60.0)
            {
                return Err(
                    "Holding route must be a closed level pattern of at least one minute".into(),
                );
            }
            legs.push(compiled);
        }
        if !segments.last().unwrap().stage.on_ground()
            && segments.last().unwrap().stage != Stage::Holding
        {
            return Err(
                "Airborne route needs a protected landing or closed holding continuation".into(),
            );
        }
        Ok(Self { segments, legs })
    }
    #[must_use]
    /// Retrieve a controller's next clearance limit.
    pub fn segment(&self, index: usize) -> Option<&Segment> {
        self.segments.get(index)
    }
    /// Exact compiled endpoint, used when planning a continuous next phase.
    #[must_use]
    pub fn end_position(&self) -> Waypoint {
        *self.segments.last().unwrap().points.last().unwrap()
    }
}

/// Immutable movement sample for conflict checks and drawing.
#[derive(Clone, Debug, Serialize)]
pub struct Pose {
    /// Stable traffic identity.
    pub id: u32,
    /// Geographic position.
    pub position: Waypoint,
    /// True ground-track degrees.
    pub heading: f64,
    /// Current ground speed.
    pub knots: f64,
    /// Current route movement.
    pub stage: Stage,
    /// Clearance-limit index, used to reject stale grants.
    pub segment: usize,
    /// This segment has not yet received movement permission.
    pub awaiting_clearance: bool,
    /// This route has reached its final clearance limit.
    pub complete: bool,
    /// The aircraft is following its separately checked recovery instead of the normal flight.
    pub recovering: bool,
    /// Nominal time remaining to this clearance limit, before any slower speed adjustment.
    pub seconds_to_limit: f64,
}

#[derive(Clone)]
struct Aircraft {
    route: std::sync::Arc<Route>,
    segment: usize,
    elapsed: f64,
    authorised: bool,
    complete: bool,
    next_authorised: bool,
    rates: Option<(f64, f64)>,
    speed: f64,
    holding: bool,
    recoveries: BTreeMap<usize, std::sync::Arc<Route>>,
    recovering: bool,
    surface_reservation: Option<(usize, BTreeSet<String>)>,
}
impl Aircraft {
    fn pose(&self, id: u32) -> Pose {
        let segment = &self.route.segments[self.segment];
        let mut elapsed = self.elapsed;
        let legs = &self.route.legs[self.segment];
        let mut position = segment.points[0];
        let mut heading = legs.first().map_or(0.0, |l| l.course);
        let mut moving = false;
        for (index, leg) in legs.iter().enumerate() {
            heading = leg.course;
            if elapsed >= leg.duration {
                elapsed -= leg.duration;
                position = leg.to;
                continue;
            }
            let fraction = elapsed / leg.duration;
            let incoming = if segment.stage == Stage::Holding && index == 0 {
                legs.last().map(|leg| leg.course)
            } else {
                index
                    .checked_sub(1)
                    .and_then(|index| legs.get(index))
                    .map(|leg| leg.course)
                    .or_else(|| {
                        self.segment
                            .checked_sub(1)
                            .and_then(|index| self.route.legs[index].last())
                            .map(|leg| leg.course)
                    })
            }
            .unwrap_or(leg.course);
            let outgoing = legs
                .get(index + 1)
                .map(|leg| leg.course)
                .or_else(|| {
                    if segment.stage == Stage::Holding {
                        legs.first().map(|leg| leg.course)
                    } else {
                        self.route
                            .legs
                            .get(self.segment + 1)
                            .and_then(|legs| legs.first())
                            .map(|leg| leg.course)
                    }
                })
                .unwrap_or(leg.course);
            let before = crate::navigation::turn(incoming, leg.course);
            let after = crate::navigation::turn(leg.course, outgoing);
            if before.abs() <= 15.0 && after.abs() <= 15.0 {
                let start = leg.course - before / 2.0;
                let end = leg.course + after / 2.0;
                heading =
                    (start + crate::navigation::turn(start, end) * fraction).rem_euclid(360.0);
            }
            if elapsed == 0.0 {
                position = leg.from;
                moving = self.authorised;
                break;
            }
            let coordinate = destination(
                leg.from.coordinate(),
                leg.course,
                segment.knots * elapsed / 3600.0,
            );
            position = Waypoint {
                latitude: coordinate[0],
                longitude: coordinate[1],
                altitude_feet: leg.from.altitude_feet
                    + (leg.to.altitude_feet - leg.from.altitude_feet) * fraction,
            };
            moving = self.authorised;
            break;
        }
        Pose {
            id,
            position,
            heading,
            knots: if moving {
                self.rates.map_or(segment.knots, |_| self.speed)
            } else {
                0.0
            },
            stage: segment.stage,
            segment: self.segment,
            awaiting_clearance: (!self.authorised || self.holding && self.speed == 0.0)
                && !self.complete,
            complete: self.complete,
            recovering: self.recovering,
            seconds_to_limit: (self.duration() - self.elapsed).max(0.0),
        }
    }
    fn duration(&self) -> f64 {
        self.route.legs[self.segment]
            .iter()
            .map(|l| l.duration)
            .sum::<f64>()
            + self.route.segments[self.segment].dwell_seconds
    }
}

/// The controller supplies whole-segment permission after checking traffic and wake rules.
/// A new segment needs its own grant or an already approved continuation.
#[derive(Default, Clone)]
pub struct World {
    generation: u64,
    aircraft: BTreeMap<u32, Aircraft>,
    owners: BTreeMap<String, u32>,
    clock: Option<f64>,
}

/// Result of accepting a simulator clock sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// First observation establishes the clock.
    Initial,
    /// Aircraft progress remains unchanged.
    Paused,
    /// An ordinary bounded time step was applied.
    Advanced,
    /// Clock discontinuity invalidated grants without moving aircraft.
    ClockChanged,
    /// The clock sample was not finite or was negative.
    Invalid,
}

impl World {
    /// Install the next planned phase at a stopped clearance limit, without granting movement.
    pub fn continue_surface(
        &mut self,
        generation: u64,
        id: u32,
        route: std::sync::Arc<Route>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        let current = aircraft.pose(id);
        let first = route.segments[0].points[0];
        if !aircraft.complete
            || !current.stage.on_ground()
            || current.knots > 0.0
            || !route.segments[0].stage.on_ground()
            || distance_nm(current.position.coordinate(), first.coordinate()) > 0.001
            || (current.position.altitude_feet - first.altitude_feet).abs() > 1.0
        {
            return false;
        }
        aircraft.route = route;
        aircraft.segment = 0;
        aircraft.elapsed = 0.0;
        aircraft.authorised = false;
        aircraft.next_authorised = false;
        aircraft.complete = false;
        aircraft.holding = false;
        aircraft.surface_reservation = None;
        aircraft.recoveries.clear();
        aircraft.recovering = false;
        true
    }
    /// Identify the reserved surface traffic preventing the next taxi clearance.
    pub fn surface_blocker(&self, id: u32) -> Option<u32> {
        let aircraft = self.aircraft.get(&id)?;
        let stage = aircraft.route.segments[aircraft.segment].stage;
        aircraft
            .route
            .segments
            .iter()
            .skip(aircraft.segment)
            .take_while(|segment| segment.stage == stage)
            .flat_map(|segment| &segment.resources)
            .filter_map(|resource| self.owners.get(resource).copied())
            .find(|owner| *owner != id)
    }
    /// Inspect the current clearance limit without exposing mutable route state.
    pub fn current_segment(&self, id: u32) -> Option<&Segment> {
        let aircraft = self.aircraft.get(&id)?;
        aircraft.route.segments.get(aircraft.segment)
    }
    /// Flight generation attached to work sent to a CPU worker.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Remove the previous flight and invalidate outstanding work.
    pub fn reset(&mut self) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            ..Self::default()
        };
    }
    /// Admit a compiled route within the active-aircraft cap.
    pub fn insert(&mut self, generation: u64, id: u32, route: std::sync::Arc<Route>) -> bool {
        if generation != self.generation
            || id == 0
            || self.aircraft.len() >= 63
            || self.aircraft.contains_key(&id)
        {
            return false;
        }
        let initial = &route.segments[0];
        if matches!(initial.stage, Stage::Parked | Stage::Turnaround) {
            if initial
                .resources
                .iter()
                .any(|resource| self.owners.contains_key(resource))
            {
                return false;
            }
            for resource in &initial.resources {
                self.owners.insert(resource.clone(), id);
            }
        }
        let initial_speed = if initial.stage.on_ground() {
            0.0
        } else {
            initial.knots
        };
        self.aircraft.insert(
            id,
            Aircraft {
                route,
                segment: 0,
                elapsed: 0.0,
                authorised: false,
                complete: false,
                next_authorised: false,
                rates: None,
                speed: initial_speed,
                holding: false,
                recoveries: BTreeMap::new(),
                recovering: false,
                surface_reservation: None,
            },
        );
        true
    }
    /// Returns false for stale, conflicting or already completed instructions.
    pub fn authorise(
        &mut self,
        generation: u64,
        id: u32,
        segment: usize,
        blocked: &BTreeSet<String>,
    ) -> bool {
        let Some(aircraft) = self.aircraft.get(&id) else {
            return false;
        };
        let stage = aircraft.route.segments[aircraft.segment].stage;
        if !stage.on_ground()
            && stage != Stage::Holding
            && !aircraft.recoveries.contains_key(&aircraft.segment)
            && !aircraft.next_authorised
        {
            return false;
        }
        if !self.reserve(generation, id, segment, blocked) {
            return false;
        }
        let aircraft = self.aircraft.get_mut(&id).unwrap();
        aircraft.authorised = true;
        aircraft.holding = false;
        true
    }
    /// Register a checked recovery before granting a movement that could wait airborne.
    /// Terrain, procedure and separation validation belongs to the controller planner.
    pub fn recovery(
        &mut self,
        generation: u64,
        id: u32,
        boundary: usize,
        route: std::sync::Arc<Route>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        if !Self::recovery_matches(&aircraft.route, boundary, &route) {
            return false;
        }
        aircraft.recoveries.insert(boundary, route);
        true
    }
    fn recovery_matches(original: &Route, boundary: usize, route: &Route) -> bool {
        let Some(current) = original.segments.get(boundary) else {
            return false;
        };
        if current.stage.on_ground() || current.stage == Stage::Holding {
            return false;
        }
        let last = current.points.last().unwrap();
        let first = route.segments[0].points[0];
        let incoming = original.legs[boundary].last().map_or(0.0, |leg| leg.course);
        let outgoing = route.legs[0].first().map_or(incoming, |leg| leg.course);
        if distance_nm(last.coordinate(), first.coordinate()) > 0.001
            || (last.altitude_feet - first.altitude_feet).abs() > 1.0
            || crate::navigation::turn(incoming, outgoing).abs() > 30.0
            || route.segments.last().unwrap().stage != Stage::Holding
            || route
                .segments
                .iter()
                .any(|segment| segment.stage.on_ground() || !segment.resources.is_empty())
        {
            return false;
        }
        true
    }
    /// Apply an approved airborne reroute without granting its later runway or taxi movements.
    pub fn replace_airborne(
        &mut self,
        generation: u64,
        id: u32,
        route: std::sync::Arc<Route>,
        recoveries: BTreeMap<usize, std::sync::Arc<Route>>,
        blocked: &BTreeSet<String>,
    ) -> bool {
        if generation != self.generation
            || route.segments[0].stage.on_ground()
            || route.segments[0].stage != Stage::Holding && !recoveries.contains_key(&0)
            || recoveries
                .iter()
                .any(|(boundary, recovery)| !Self::recovery_matches(&route, *boundary, recovery))
        {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        if !aircraft.authorised
            || aircraft.complete
            || aircraft.route.segments[aircraft.segment].stage.on_ground()
        {
            return false;
        }
        let current = aircraft.pose(id);
        let first = &route.segments[0];
        let outgoing = route.legs[0]
            .first()
            .map_or(current.heading, |leg| leg.course);
        if distance_nm(current.position.coordinate(), first.points[0].coordinate()) > 0.001
            || (current.position.altitude_feet - first.points[0].altitude_feet).abs() > 1.0
            || crate::navigation::turn(current.heading, outgoing).abs() > 30.0
            || first.knots + 0.1 < aircraft.speed
            || first.resources.iter().any(|resource| {
                blocked.contains(resource)
                    || self.owners.get(resource).is_some_and(|owner| *owner != id)
            })
        {
            return false;
        }
        self.owners.retain(|_, owner| *owner != id);
        for resource in &first.resources {
            self.owners.insert(resource.clone(), id);
        }
        aircraft.route = route;
        aircraft.segment = 0;
        aircraft.elapsed = 0.0;
        aircraft.authorised = true;
        aircraft.next_authorised = false;
        aircraft.complete = false;
        aircraft.holding = false;
        aircraft.surface_reservation = None;
        aircraft.recovering = false;
        aircraft.recoveries = recoveries;
        true
    }
    /// Replace a hold only after the controller approves a complete onward path.
    pub fn leave_hold(&mut self, generation: u64, id: u32, route: std::sync::Arc<Route>) -> bool {
        if generation != self.generation
            || route.segments.last().unwrap().stage != Stage::Holding
            || route
                .segments
                .iter()
                .any(|segment| segment.stage.on_ground() || !segment.resources.is_empty())
        {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        if aircraft.route.segments[aircraft.segment].stage != Stage::Holding {
            return false;
        }
        let current = aircraft.pose(id);
        let first = route.segments[0].points[0];
        let outgoing = route.legs[0]
            .first()
            .map_or(current.heading, |leg| leg.course);
        if distance_nm(current.position.coordinate(), first.coordinate()) > 0.001
            || (current.position.altitude_feet - first.altitude_feet).abs() > 1.0
            || crate::navigation::turn(current.heading, outgoing).abs() > 30.0
        {
            return false;
        }
        aircraft.route = route;
        aircraft.segment = 0;
        aircraft.elapsed = 0.0;
        aircraft.authorised = true;
        aircraft.next_authorised = false;
        aircraft.complete = false;
        aircraft.recovering = true;
        aircraft.recoveries.clear();
        self.owners.retain(|_, owner| *owner != id);
        true
    }
    /// A separately approved onward segment may be reserved before reaching its boundary.
    pub fn authorise_continuation(
        &mut self,
        generation: u64,
        id: u32,
        segment: usize,
        blocked: &BTreeSet<String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        if segment != aircraft.segment + 1 || !aircraft.authorised || aircraft.holding {
            return false;
        }
        let Some(next) = aircraft.route.segments.get(segment) else {
            return false;
        };
        if let Some((_, braking)) = aircraft.rates {
            let remaining = (aircraft.duration() - aircraft.elapsed)
                * aircraft.route.segments[aircraft.segment].knots
                * 0.514444;
            let slowing_distance =
                (aircraft.speed.powi(2) - next.knots.powi(2)).max(0.0) * 0.514444 / (2.0 * braking);
            if remaining + 0.02 < slowing_distance {
                return false;
            }
        }
        if !next.stage.on_ground()
            && next.stage != Stage::Holding
            && !aircraft.recoveries.contains_key(&segment)
        {
            return false;
        }
        if next.resources.iter().any(|resource| {
            blocked.contains(resource)
                || self.owners.get(resource).is_some_and(|owner| *owner != id)
        }) {
            return false;
        }
        for resource in &next.resources {
            self.owners.insert(resource.clone(), id);
        }
        aircraft.next_authorised = true;
        true
    }
    /// Protect a clearance while its readback is pending, without moving the aircraft.
    pub fn reserve_surface_route(
        &mut self,
        generation: u64,
        id: u32,
        segment: usize,
        blocked: &BTreeSet<String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        if aircraft.segment != segment || aircraft.complete {
            return false;
        }
        let stage = aircraft.route.segments[segment].stage;
        if !matches!(stage, Stage::TaxiOut | Stage::TaxiIn | Stage::Pushback) {
            return self.reserve(generation, id, segment, blocked);
        }
        let mut end = segment;
        let mut resources = BTreeSet::new();
        for (index, next) in aircraft.route.segments.iter().enumerate().skip(segment) {
            if next.stage != stage {
                break;
            }
            end = index;
            resources.extend(next.resources.iter().cloned());
        }
        if resources.iter().any(|resource| {
            blocked.contains(resource)
                || self.owners.get(resource).is_some_and(|owner| *owner != id)
        }) {
            return false;
        }
        for resource in &resources {
            self.owners.insert(resource.clone(), id);
        }
        aircraft.surface_reservation = Some((end, resources));
        true
    }

    /// Reserve one protected segment without granting movement.
    pub fn reserve(
        &mut self,
        generation: u64,
        id: u32,
        segment: usize,
        blocked: &BTreeSet<String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(a) = self.aircraft.get_mut(&id) else {
            return false;
        };
        if a.segment != segment || a.complete {
            return false;
        }
        let resources = &a.route.segments[segment].resources;
        if resources
            .iter()
            .any(|r| blocked.contains(r) || self.owners.get(r).is_some_and(|owner| *owner != id))
        {
            return false;
        }
        for resource in resources {
            self.owners.insert(resource.clone(), id);
        }
        true
    }
    /// Withdraw an unexecuted surface grant while retaining the aircraft's occupied cells.
    pub fn cancel_reservation(
        &mut self,
        generation: u64,
        id: u32,
        occupied: &BTreeSet<String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        if !aircraft.route.segments[aircraft.segment].stage.on_ground() || aircraft.speed > 0.0 {
            return false;
        }
        aircraft.authorised = false;
        aircraft.next_authorised = false;
        aircraft.surface_reservation = None;
        self.owners
            .retain(|resource, owner| *owner != id || occupied.contains(resource));
        for resource in occupied {
            self.owners.entry(resource.clone()).or_insert(id);
        }
        true
    }
    /// Brake surface traffic when a new observed obstruction enters its reserved path.
    /// Unknown observations suspend movement; they never imply an empty airport.
    pub fn recheck_surface(&mut self, blocked: &BTreeSet<String>, observations_known: bool) {
        for (id, aircraft) in &mut self.aircraft {
            let current = &aircraft.route.segments[aircraft.segment];
            if !matches!(
                current.stage,
                Stage::TaxiOut | Stage::TaxiIn | Stage::Pushback
            ) {
                continue;
            }
            aircraft.holding = !observations_known
                || current
                    .resources
                    .iter()
                    .any(|resource| blocked.contains(resource));
            if aircraft.holding
                || aircraft
                    .route
                    .segments
                    .get(aircraft.segment + 1)
                    .is_some_and(|next| {
                        next.resources
                            .iter()
                            .any(|resource| blocked.contains(resource))
                    })
            {
                aircraft.next_authorised = false;
                self.owners.retain(|resource, owner| {
                    *owner != *id
                        || current.resources.contains(resource)
                        || aircraft
                            .surface_reservation
                            .as_ref()
                            .is_some_and(|(_, reserved)| reserved.contains(resource))
                });
            }
        }
    }
    /// Set per-aircraft acceleration and braking from a validated profile.
    pub fn ground_rates(&mut self, id: u32, acceleration: f64, braking: f64) -> bool {
        if !(0.1..=10.0).contains(&acceleration) || !(0.1..=10.0).contains(&braking) {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        aircraft.rates = Some((acceleration, braking));
        true
    }
    /// Reserve one following taxi leg to avoid stopping at every graph node.
    /// Runway entry, crossing and takeoff require their own controller grant.
    pub fn reserve_next_taxi(
        &mut self,
        generation: u64,
        id: u32,
        blocked: &BTreeSet<String>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        let Some(aircraft) = self.aircraft.get_mut(&id) else {
            return false;
        };
        let next = aircraft.segment + 1;
        let Some(segment) = aircraft.route.segments.get(next) else {
            return false;
        };
        if !aircraft.authorised
            || aircraft.holding
            || !matches!(segment.stage, Stage::TaxiOut | Stage::TaxiIn)
            || !matches!(
                aircraft.route.segments[aircraft.segment].stage,
                Stage::TaxiOut | Stage::TaxiIn
            )
            || segment.resources.iter().any(|resource| {
                blocked.contains(resource)
                    || self.owners.get(resource).is_some_and(|owner| *owner != id)
            })
        {
            return false;
        }
        for resource in &segment.resources {
            self.owners.insert(resource.clone(), id);
        }
        aircraft.next_authorised = true;
        true
    }
    /// Pausing and large clock changes cannot move traffic through a clearance limit.
    pub fn advance(&mut self, seconds: f64, paused: bool) -> Advance {
        if !seconds.is_finite() || seconds < 0.0 {
            return Advance::Invalid;
        }
        let previous = self.clock.replace(seconds);
        if paused {
            return Advance::Paused;
        }
        let Some(previous) = previous else {
            return Advance::Initial;
        };
        let elapsed = seconds - previous;
        if !(0.0..=30.0).contains(&elapsed) {
            // Keep aircraft and occupied resources; require the controller to reconsider each grant.
            for aircraft in self.aircraft.values_mut() {
                if aircraft.route.segments[aircraft.segment].stage.on_ground() {
                    aircraft.authorised = false;
                    aircraft.speed = 0.0;
                }
                aircraft.next_authorised = false;
            }
            return Advance::ClockChanged;
        }
        let steps = (elapsed / 0.5).ceil().max(1.0) as usize;
        let elapsed = elapsed / steps as f64;
        for _ in 0..steps {
            for (id, aircraft) in &mut self.aircraft {
                if !aircraft.authorised || aircraft.complete {
                    continue;
                }
                let segment = &aircraft.route.segments[aircraft.segment];
                let progress = if let Some((acceleration, braking)) = aircraft.rates
                    && segment.knots > 0.0
                {
                    let remaining =
                        (aircraft.duration() - aircraft.elapsed) * segment.knots * 0.514444;
                    let next_speed = if aircraft.next_authorised || aircraft.recovering {
                        aircraft
                            .route
                            .segments
                            .get(aircraft.segment + 1)
                            .map(|segment| segment.knots)
                    } else {
                        aircraft
                            .recoveries
                            .get(&aircraft.segment)
                            .map(|route| route.segments[0].knots)
                    };
                    let distance_to_limit = remaining
                        + if aircraft.next_authorised {
                            let index = aircraft.segment + 1;
                            aircraft.route.legs[index]
                                .iter()
                                .map(|leg| leg.duration)
                                .sum::<f64>()
                                * aircraft.route.segments[index].knots
                                * 0.514444
                        } else {
                            0.0
                        };
                    let nonstop = aircraft.recovering
                        || segment.stage == Stage::Holding
                        || !segment.stage.on_ground()
                            && aircraft.recoveries.contains_key(&aircraft.segment);
                    let target = if aircraft.holding && segment.stage.on_ground() {
                        0.0
                    } else {
                        let limit = if nonstop {
                            segment.knots
                        } else {
                            (2.0 * braking * 0.514444 * distance_to_limit).sqrt() / 0.514444
                        };
                        let next_limit = next_speed.map_or(segment.knots, |speed| {
                            (speed.powi(2) + 2.0 * braking * remaining / 0.514444).sqrt()
                        });
                        segment.knots.min(limit).min(next_limit)
                    };
                    let before = aircraft.speed;
                    aircraft.speed +=
                        (target - before).clamp(-braking * elapsed, acceleration * elapsed);
                    if remaining < 0.02 {
                        aircraft.duration() - aircraft.elapsed
                    } else {
                        elapsed * (before + aircraft.speed) / 2.0 / segment.knots
                    }
                } else {
                    elapsed
                };
                let total = aircraft.elapsed + progress;
                let duration = aircraft.duration();
                aircraft.elapsed = total.min(duration);
                if aircraft.elapsed >= duration && segment.stage == Stage::Holding {
                    aircraft.elapsed = total.rem_euclid(duration);
                    continue;
                }
                if aircraft.elapsed >= aircraft.duration() {
                    if !aircraft.next_authorised
                        && !segment.stage.on_ground()
                        && let Some(recovery) = aircraft.recoveries.remove(&aircraft.segment)
                    {
                        self.owners.retain(|_, owner| *owner != *id);
                        aircraft.route = recovery;
                        aircraft.segment = 0;
                        aircraft.elapsed = (total - duration).min(aircraft.duration());
                        aircraft.authorised = true;
                        aircraft.complete = false;
                        aircraft.recoveries.clear();
                        aircraft.recovering = true;
                        continue;
                    }
                    let next = aircraft.segment + 1;
                    // Retain shared resources until the segment that actually leaves them completes.
                    let retained = aircraft
                        .route
                        .segments
                        .get(next)
                        .map(|s| &s.resources)
                        .or_else(|| {
                            aircraft.route.segments[aircraft.segment]
                                .stage
                                .on_ground()
                                .then_some(&aircraft.route.segments[aircraft.segment].resources)
                        });
                    self.owners.retain(|resource, owner| {
                        *owner != *id
                            || retained.is_some_and(|r| r.contains(resource))
                            || aircraft.surface_reservation.as_ref().is_some_and(
                                |(end, resources)| next <= *end && resources.contains(resource),
                            )
                    });
                    if aircraft
                        .surface_reservation
                        .as_ref()
                        .is_some_and(|(end, _)| next > *end)
                    {
                        aircraft.surface_reservation = None;
                    }
                    aircraft.authorised = aircraft.next_authorised || aircraft.recovering;
                    aircraft.next_authorised = false;
                    if next == aircraft.route.segments.len() {
                        aircraft.complete = true;
                    } else {
                        aircraft.segment = next;
                        aircraft.elapsed = 0.0;
                        if aircraft.authorised {
                            aircraft.elapsed = (total - duration).min(aircraft.duration());
                        }
                    }
                }
            }
        }
        Advance::Advanced
    }
    #[must_use]
    /// Publish at most 63 complete poses without exposing mutable movement state.
    pub fn snapshot(&self) -> Vec<Pose> {
        self.aircraft.iter().map(|(id, a)| a.pose(*id)).collect()
    }
    /// Density changes retire completed flights; they never remove active runway traffic.
    pub fn retire(&mut self, id: u32) -> bool {
        if !self.aircraft.get(&id).is_some_and(|a| a.complete) {
            return false;
        }
        self.aircraft.remove(&id);
        self.owners.retain(|_, owner| *owner != id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(meters: f64, altitude: f64) -> Waypoint {
        Waypoint {
            latitude: 0.0,
            longitude: meters / 111_320.0,
            altitude_feet: altitude,
        }
    }
    fn crossing() -> std::sync::Arc<Route> {
        std::sync::Arc::new(
            Route::compile(
                vec![
                    Segment {
                        stage: Stage::Crossing,
                        points: vec![point(0.0, 0.0), point(100.0, 0.0)],
                        knots: 10.0,
                        resources: vec!["TEST/09-27".into()],
                        dwell_seconds: 0.0,
                    },
                    Segment {
                        stage: Stage::TaxiIn,
                        points: vec![point(100.0, 0.0), point(200.0, 0.0)],
                        knots: 10.0,
                        resources: vec![],
                        dwell_seconds: 0.0,
                    },
                ],
                1000.0,
                1000.0,
            )
            .unwrap(),
        )
    }
    #[test]
    fn taxi_corridor_waits_before_entering_a_shared_lane() {
        let route = |start: &str| {
            std::sync::Arc::new(
                Route::compile(
                    vec![
                        Segment {
                            stage: Stage::TaxiOut,
                            points: vec![point(0.0, 0.0), point(100.0, 0.0)],
                            knots: 10.0,
                            resources: vec![start.into()],
                            dwell_seconds: 0.0,
                        },
                        Segment {
                            stage: Stage::TaxiOut,
                            points: vec![point(100.0, 0.0), point(200.0, 0.0)],
                            knots: 10.0,
                            resources: vec!["SHARED".into()],
                            dwell_seconds: 0.0,
                        },
                        Segment {
                            stage: Stage::Crossing,
                            points: vec![point(200.0, 0.0), point(300.0, 0.0)],
                            knots: 10.0,
                            resources: vec!["RUNWAY".into()],
                            dwell_seconds: 0.0,
                        },
                    ],
                    1000.0,
                    1000.0,
                )
                .unwrap(),
            )
        };
        let mut world = World::default();
        assert!(world.insert(0, 1, route("RAMP1")));
        assert!(world.insert(0, 2, route("RAMP2")));
        assert!(world.reserve_surface_route(0, 1, 0, &BTreeSet::new()));
        assert!(!world.reserve_surface_route(0, 2, 0, &BTreeSet::new()));
        assert!(!world.owners.contains_key("RAMP2"));
        assert!(!world.owners.contains_key("RUNWAY"));
        assert!(world.authorise(0, 1, 0, &BTreeSet::new()));
        assert!(world.reserve_next_taxi(0, 1, &BTreeSet::new()));
        world.advance(0.0, false);
        world.advance(25.0, false);
        assert_eq!(world.snapshot()[0].segment, 1);
        assert_eq!(world.owners.get("SHARED"), Some(&1));
        world.advance(50.0, false);
        assert_eq!(world.snapshot()[0].stage, Stage::Crossing);
        assert!(world.snapshot()[0].awaiting_clearance);
        assert!(world.reserve_surface_route(0, 2, 0, &BTreeSet::new()));
    }
    #[test]
    fn departure_phase_keeps_the_hold_until_a_new_controller_grant() {
        let mut world = World::default();
        let taxi = std::sync::Arc::new(
            Route::compile(
                vec![Segment {
                    stage: Stage::TaxiOut,
                    points: vec![point(0.0, 0.0), point(100.0, 0.0)],
                    knots: 10.0,
                    resources: vec!["HOLD".into()],
                    dwell_seconds: 0.0,
                }],
                1000.0,
                1000.0,
            )
            .unwrap(),
        );
        let departure = |start| {
            std::sync::Arc::new(
                Route::compile(
                    vec![Segment {
                        stage: Stage::Takeoff,
                        points: vec![point(start, 0.0), point(1000.0, 0.0)],
                        knots: 60.0,
                        resources: vec!["RUNWAY".into()],
                        dwell_seconds: 0.0,
                    }],
                    1000.0,
                    1000.0,
                )
                .unwrap(),
            )
        };
        world.insert(0, 1, taxi);
        assert!(!world.continue_surface(0, 1, departure(100.0)));
        world.authorise(0, 1, 0, &BTreeSet::new());
        world.advance(0.0, false);
        world.advance(25.0, false);
        assert!(!world.continue_surface(0, 1, departure(200.0)));
        assert!(world.continue_surface(0, 1, departure(100.0)));
        assert_eq!(world.owners.get("HOLD"), Some(&1));
        assert!(!world.owners.contains_key("RUNWAY"));
        world.advance(26.0, false);
        assert_eq!(world.snapshot()[0].position, point(100.0, 0.0));
        assert!(world.snapshot()[0].awaiting_clearance);
        assert!(!world.authorise(0, 1, 0, &BTreeSet::from(["RUNWAY".into()])));
        assert!(world.authorise(0, 1, 0, &BTreeSet::new()));
        world.advance(27.0, false);
        assert!(world.snapshot()[0].position.longitude > point(100.0, 0.0).longitude);
    }
    #[test]
    fn airborne_reroute_requires_recovery_and_keeps_landing_and_taxi_grants_separate() {
        let hold = |fix: Waypoint, heading| {
            let segment = crate::traffic::holding::Pattern {
                fix,
                inbound_degrees: heading,
                right: true,
                ground_knots: 100.0,
                leg_seconds: 60.0,
                bank_degrees: 25.0,
                turn_rate_degrees_sec: 3.0,
            }
            .compile(|_, _| true)
            .unwrap();
            std::sync::Arc::new(Route::compile(vec![segment], 1000.0, 1000.0).unwrap())
        };
        let fix = Waypoint {
            latitude: 0.0,
            longitude: 0.0,
            altitude_feet: 1500.0,
        };
        let mut world = World::default();
        world.insert(0, 1, hold(fix, 90.0));
        world.ground_rates(1, 1.0, 5.0);
        world.authorise(0, 1, 0, &BTreeSet::new());
        world.advance(0.0, false);
        world.advance(1.0, false);
        let current = world.snapshot()[0].clone();
        let next = |from: Waypoint, meters, height| {
            let coordinate = destination(from.coordinate(), current.heading, meters / 1852.0);
            Waypoint {
                latitude: coordinate[0],
                longitude: coordinate[1],
                altitude_feet: height,
            }
        };
        let touchdown = next(current.position, 1852.0, 1000.0);
        let exit = next(touchdown, 1000.0, 1000.0);
        let gate = next(exit, 200.0, 1000.0);
        let route = std::sync::Arc::new(
            Route::compile(
                vec![
                    Segment {
                        stage: Stage::Approach,
                        points: vec![current.position, touchdown],
                        knots: 100.0,
                        resources: vec![],
                        dwell_seconds: 0.0,
                    },
                    Segment {
                        stage: Stage::Rollout,
                        points: vec![touchdown, exit],
                        knots: 100.0,
                        resources: vec!["RUNWAY".into()],
                        dwell_seconds: 0.0,
                    },
                    Segment {
                        stage: Stage::TaxiIn,
                        points: vec![exit, gate],
                        knots: 10.0,
                        resources: vec!["TAXI".into()],
                        dwell_seconds: 0.0,
                    },
                ],
                1000.0,
                1000.0,
            )
            .unwrap(),
        );
        assert!(!world.replace_airborne(0, 1, route.clone(), BTreeMap::new(), &BTreeSet::new()));
        assert_eq!(world.snapshot()[0].stage, Stage::Holding);
        let recovery = BTreeMap::from([(0, hold(touchdown, current.heading))]);
        let blocked = BTreeSet::from(["RUNWAY".into()]);
        assert!(world.replace_airborne(0, 1, route, recovery, &blocked));
        assert!(!world.owners.contains_key("RUNWAY"));
        assert!(!world.authorise_continuation(0, 1, 1, &blocked));
        let mut cleared = world.clone();
        assert!(cleared.authorise_continuation(0, 1, 1, &BTreeSet::new()));
        for second in 2..=100 {
            world.advance(f64::from(second), false);
            cleared.advance(f64::from(second), false);
        }
        let refused = &world.snapshot()[0];
        assert_eq!(refused.stage, Stage::Holding);
        assert!(!refused.awaiting_clearance);
        assert!(!refused.stage.on_ground());
        let landed = &cleared.snapshot()[0];
        assert_eq!(landed.stage, Stage::TaxiIn);
        assert!(landed.awaiting_clearance);
        assert_eq!(landed.position, exit);
        assert!(!cleared.owners.contains_key("RUNWAY"));
    }

    #[test]
    fn airborne_spawn_and_late_speed_clearance_keep_the_recovery_flyable() {
        let start = Waypoint {
            latitude: 0.0,
            longitude: 0.0,
            altitude_feet: 5000.0,
        };
        let entry = Waypoint {
            longitude: 0.1,
            ..start
        };
        let end = Waypoint {
            longitude: 0.3,
            altitude_feet: 3000.0,
            ..start
        };
        let hold = |fix, knots| {
            crate::traffic::holding::Pattern {
                fix,
                inbound_degrees: 90.0,
                right: true,
                ground_knots: knots,
                leg_seconds: 60.0,
                bank_degrees: 25.0,
                turn_rate_degrees_sec: 3.0,
            }
            .compile(|_, _| true)
            .unwrap()
        };
        let route = std::sync::Arc::new(
            Route::compile(
                vec![
                    Segment {
                        stage: Stage::Cruise,
                        points: vec![start, entry],
                        knots: 200.0,
                        resources: vec![],
                        dwell_seconds: 0.0,
                    },
                    Segment {
                        stage: Stage::Descent,
                        points: vec![entry, end],
                        knots: 100.0,
                        resources: vec![],
                        dwell_seconds: 0.0,
                    },
                    hold(end, 100.0),
                ],
                2000.0,
                2000.0,
            )
            .unwrap(),
        );
        let prepare = || {
            let mut world = World::default();
            assert!(world.insert(0, 1, route.clone()));
            assert!(world.ground_rates(1, 1.0, 1.0));
            for (boundary, fix, knots) in [(0, entry, 200.0), (1, end, 100.0)] {
                let recovery = std::sync::Arc::new(
                    Route::compile(vec![hold(fix, knots)], 2000.0, 2000.0).unwrap(),
                );
                assert!(world.recovery(0, 1, boundary, recovery));
            }
            assert!(world.authorise(0, 1, 0, &BTreeSet::new()));
            assert_eq!(world.snapshot()[0].knots, 200.0);
            world.advance(0.0, false);
            world
        };
        let mut early = prepare();
        assert!(early.authorise_continuation(0, 1, 1, &BTreeSet::new()));
        let mut entered = false;
        for second in 1..=180 {
            early.advance(f64::from(second), false);
            let pose = &early.snapshot()[0];
            if pose.stage == Stage::Descent {
                assert!(
                    pose.knots <= 101.0,
                    "Descent entered before slowing: {}",
                    pose.knots
                );
                entered = true;
                break;
            }
        }
        assert!(entered);
        let mut late = prepare();
        let duration = late.aircraft[&1].duration();
        for second in 1..=duration.floor() as u32 {
            late.advance(f64::from(second), false);
        }
        assert!(!late.authorise_continuation(0, 1, 1, &BTreeSet::new()));
        late.advance(duration + 5.0, false);
        let pose = &late.snapshot()[0];
        assert_eq!(pose.stage, Stage::Holding);
        assert!(!pose.awaiting_clearance);
        assert!(pose.knots > 190.0);
    }

    #[test]
    fn crossing_requires_a_grant_and_continuation_requires_another() {
        let mut w = World::default();
        assert!(w.insert(0, 1, crossing()));
        assert!(w.insert(0, 2, crossing()));
        assert_eq!(w.advance(100.0, false), Advance::Initial);
        w.advance(110.0, false);
        assert_eq!(w.snapshot()[0].position, point(0.0, 0.0));
        assert!(!w.authorise(0, 1, 0, &BTreeSet::from(["TEST/09-27".into()])));
        assert!(w.authorise(0, 1, 0, &BTreeSet::new()));
        assert!(!w.authorise(0, 2, 0, &BTreeSet::new()));
        w.advance(135.0, false);
        let pose = &w.snapshot()[0];
        assert_eq!(pose.segment, 1);
        assert!(pose.awaiting_clearance);
        assert_eq!(pose.position, point(100.0, 0.0));
        assert!(w.authorise(0, 2, 0, &BTreeSet::new()));
        assert!(!w.authorise(0, 1, 0, &BTreeSet::new()));
        assert!(!w.retire(1));
    }
    #[test]
    fn pause_clock_jump_and_reset_never_apply_old_work() {
        let mut w = World::default();
        w.insert(0, 1, crossing());
        w.authorise(0, 1, 0, &BTreeSet::new());
        w.advance(100.0, false);
        w.advance(110.0, true);
        assert_eq!(w.snapshot()[0].position, point(0.0, 0.0));
        assert_eq!(w.advance(1000.0, false), Advance::ClockChanged);
        w.advance(1001.0, false);
        assert_eq!(w.snapshot()[0].position, point(0.0, 0.0));
        w.reset();
        assert!(!w.insert(0, 2, crossing()));
        assert!(!w.authorise(0, 1, 0, &BTreeSet::new()));
        assert!(w.snapshot().is_empty());
    }
    #[test]
    fn taxi_brakes_for_new_obstruction_and_does_not_reserve_a_crossing() {
        let route = Route::compile(
            vec![
                Segment {
                    stage: Stage::TaxiOut,
                    points: vec![point(0.0, 0.0), point(100.0, 0.0)],
                    knots: 10.0,
                    resources: vec!["A".into()],
                    dwell_seconds: 0.0,
                },
                Segment {
                    stage: Stage::TaxiOut,
                    points: vec![point(100.0, 0.0), point(200.0, 0.0)],
                    knots: 10.0,
                    resources: vec!["B".into()],
                    dwell_seconds: 0.0,
                },
                Segment {
                    stage: Stage::Crossing,
                    points: vec![point(200.0, 0.0), point(300.0, 0.0)],
                    knots: 10.0,
                    resources: vec!["RUNWAY".into()],
                    dwell_seconds: 0.0,
                },
            ],
            1000.0,
            1000.0,
        )
        .unwrap();
        let mut world = World::default();
        assert!(world.insert(0, 1, std::sync::Arc::new(route)));
        assert!(world.ground_rates(1, 2.0, 2.0));
        assert!(world.authorise(0, 1, 0, &BTreeSet::new()));
        assert!(world.reserve_next_taxi(0, 1, &BTreeSet::new()));
        world.advance(0.0, false);
        for second in 1..=5 {
            world.advance(f64::from(second), false);
        }
        let before = world.snapshot()[0].clone();
        assert_eq!(before.knots, 10.0);
        world.recheck_surface(&BTreeSet::from(["A".into()]), true);
        world.advance(6.0, false);
        let braking = world.snapshot()[0].clone();
        assert_eq!(braking.knots, 8.0);
        assert!(braking.position.longitude > before.position.longitude);
        for second in 7..=12 {
            world.advance(f64::from(second), false);
        }
        assert_eq!(world.snapshot()[0].knots, 0.0);
        let stopped = world.snapshot()[0].position;
        world.advance(13.0, false);
        assert_eq!(world.snapshot()[0].position, stopped);
        world.recheck_surface(&BTreeSet::new(), true);
        assert!(world.authorise(0, 1, 0, &BTreeSet::new()));
        assert!(world.reserve_next_taxi(0, 1, &BTreeSet::new()));
        let mut continued_at_speed = false;
        for second in 14..=80 {
            world.advance(f64::from(second), false);
            let pose = world.snapshot()[0].clone();
            if pose.segment == 1 {
                continued_at_speed |= pose.knots > 5.0;
            }
            assert!(!world.reserve_next_taxi(0, 1, &BTreeSet::new()) || pose.segment == 0);
            if pose.segment == 2 {
                assert!(pose.awaiting_clearance);
                assert_eq!(pose.position, point(200.0, 0.0));
                break;
            }
        }
        assert!(continued_at_speed);
        assert_eq!(world.snapshot()[0].segment, 2);
    }
    #[test]
    fn occupied_final_holding_point_is_not_released_until_retirement() {
        let route = std::sync::Arc::new(
            Route::compile(
                vec![Segment {
                    stage: Stage::TaxiOut,
                    points: vec![point(0.0, 0.0), point(10.0, 0.0)],
                    knots: 10.0,
                    resources: vec!["HOLD".into()],
                    dwell_seconds: 0.0,
                }],
                1000.0,
                1000.0,
            )
            .unwrap(),
        );
        let mut world = World::default();
        world.insert(0, 1, route.clone());
        world.insert(0, 2, route);
        assert!(world.authorise(0, 1, 0, &BTreeSet::new()));
        world.advance(0.0, false);
        world.advance(10.0, false);
        assert!(world.snapshot()[0].complete);
        assert!(!world.authorise(0, 2, 0, &BTreeSet::new()));
        assert!(world.retire(1));
        assert!(world.authorise(0, 2, 0, &BTreeSet::new()));
    }
    #[test]
    fn an_unavailable_landing_uses_its_cleared_hold_instead_of_stopping_airborne() {
        let holding = |anchor: Waypoint, course: f64| {
            let mut points = Vec::new();
            for index in 0..=36 {
                let angle = (f64::from(index) * 10.0).to_radians();
                let along = 500.0 * angle.sin();
                let right = 500.0 * (1.0 - angle.cos());
                let radians = course.to_radians();
                points.push(Waypoint {
                    latitude: anchor.latitude
                        + (along * radians.cos() - right * radians.sin()) / 111320.0,
                    longitude: anchor.longitude
                        + (along * radians.sin() + right * radians.cos()) / 111320.0,
                    altitude_feet: anchor.altitude_feet,
                });
            }
            std::sync::Arc::new(
                Route::compile(
                    vec![Segment {
                        stage: Stage::Holding,
                        points,
                        knots: 80.0,
                        resources: vec![],
                        dwell_seconds: 0.0,
                    }],
                    1000.0,
                    1000.0,
                )
                .unwrap(),
            )
        };
        let route = std::sync::Arc::new(
            Route::compile(
                vec![
                    Segment {
                        stage: Stage::Cruise,
                        points: vec![point(0.0, 5000.0), point(1000.0, 5000.0)],
                        knots: 100.0,
                        resources: vec![],
                        dwell_seconds: 0.0,
                    },
                    Segment {
                        stage: Stage::Rollout,
                        points: vec![point(1000.0, 5000.0), point(1010.0, 5000.0)],
                        knots: 10.0,
                        resources: vec!["RW".into()],
                        dwell_seconds: 0.0,
                    },
                ],
                1000.0,
                1000.0,
            )
            .unwrap(),
        );
        let mut world = World::default();
        world.insert(0, 1, route);
        assert!(
            !world.authorise(0, 1, 0, &BTreeSet::new()),
            "An airborne clearance needs a recovery"
        );
        assert!(world.recovery(0, 1, 0, holding(point(1000.0, 5000.0), 90.0)));
        assert!(world.authorise(0, 1, 0, &BTreeSet::new()));
        assert!(!world.authorise_continuation(0, 1, 1, &BTreeSet::from(["RW".into()])));
        world.advance(0.0, false);
        world.advance(20.0, false);
        let first = world.snapshot()[0].clone();
        assert_eq!(first.stage, Stage::Holding);
        world.advance(40.0, false);
        let second = world.snapshot()[0].clone();
        assert_ne!(first.position, second.position);
        assert_eq!(second.knots, 80.0);
        assert!(!second.complete && !second.awaiting_clearance);
        assert!(!world.leave_hold(1, 1, holding(second.position, second.heading)));
        assert!(!world.leave_hold(0, 1, holding(point(0.0, 5000.0), second.heading)));
        assert!(world.leave_hold(0, 1, holding(second.position, second.heading)));
        world.advance(60.0, false);
        assert_ne!(world.snapshot()[0].position, second.position);
        world.advance(1000.0, false);
        let clock_changed = world.snapshot()[0].position;
        world.advance(1001.0, false);
        assert_ne!(world.snapshot()[0].position, clock_changed);
    }
    #[test]
    fn impossible_climb_and_route_gaps_are_rejected() {
        let route = Segment {
            stage: Stage::Climb,
            points: vec![point(0.0, 0.0), point(100.0, 10000.0)],
            knots: 150.0,
            resources: vec![],
            dwell_seconds: 0.0,
        };
        assert!(
            Route::compile(vec![route], 2000.0, 2000.0)
                .unwrap_err()
                .contains("performance")
        );
        let mut segments = crossing().segments.clone();
        segments[1].points[0] = point(500.0, 0.0);
        assert!(
            Route::compile(segments, 2000.0, 2000.0)
                .unwrap_err()
                .contains("discontinuity")
        );
    }
}
