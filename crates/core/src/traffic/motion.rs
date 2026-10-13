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
    /// Takeoff roll, ending at rotation.
    Takeoff,
    /// Published or cleared climb path.
    Climb,
    /// Cleared en-route legs.
    Cruise,
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
            || segments.len() > 128
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
                || segment.resources.len() > 8
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
                Stage::Crossing | Stage::Takeoff | Stage::Rollout
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
                if vertical > climb_fpm || -vertical > descent_fpm {
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
            legs.push(compiled);
        }
        Ok(Self { segments, legs })
    }
    #[must_use]
    /// Retrieve a controller's next clearance limit.
    pub fn segment(&self, index: usize) -> Option<&Segment> {
        self.segments.get(index)
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
    /// Final stand dwell has completed.
    pub complete: bool,
}

#[derive(Clone)]
struct Aircraft {
    route: std::sync::Arc<Route>,
    segment: usize,
    elapsed: f64,
    authorised: bool,
    complete: bool,
}
impl Aircraft {
    fn pose(&self, id: u32) -> Pose {
        let segment = &self.route.segments[self.segment];
        let mut elapsed = self.elapsed;
        let legs = &self.route.legs[self.segment];
        let mut position = segment.points[0];
        let mut heading = legs.first().map_or(0.0, |l| l.course);
        let mut moving = false;
        for leg in legs {
            heading = leg.course;
            if elapsed >= leg.duration {
                elapsed -= leg.duration;
                position = leg.to;
                continue;
            }
            let fraction = elapsed / leg.duration;
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
            knots: if moving { segment.knots } else { 0.0 },
            stage: segment.stage,
            segment: self.segment,
            awaiting_clearance: !self.authorised && !self.complete,
            complete: self.complete,
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
/// A grant never carries across to the next segment or a replacement flight generation.
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
        self.aircraft.insert(
            id,
            Aircraft {
                route,
                segment: 0,
                elapsed: 0.0,
                authorised: false,
                complete: false,
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
        a.authorised = true;
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
                aircraft.authorised = false;
            }
            return Advance::ClockChanged;
        }
        for (id, aircraft) in &mut self.aircraft {
            if !aircraft.authorised || aircraft.complete {
                continue;
            }
            aircraft.elapsed = (aircraft.elapsed + elapsed).min(aircraft.duration());
            if aircraft.elapsed >= aircraft.duration() {
                let next = aircraft.segment + 1;
                // Retain shared resources until the segment that actually leaves them completes.
                let retained = aircraft.route.segments.get(next).map(|s| &s.resources);
                self.owners.retain(|resource, owner| {
                    *owner != *id || retained.is_some_and(|r| r.contains(resource))
                });
                aircraft.authorised = false;
                if next == aircraft.route.segments.len() {
                    aircraft.complete = true;
                } else {
                    aircraft.segment = next;
                    aircraft.elapsed = 0.0;
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
