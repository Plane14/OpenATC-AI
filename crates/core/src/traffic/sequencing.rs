//! Same-final arrival spacing, shared by observed and generated aircraft.
//!
//! This is a prediction, never a landing clearance. Runway occupancy, wake observation,
//! protected routing and the radio readback remain independent controller checks.
use super::{profiles::Aircraft, wake};

/// Regions whose speed-control and arrival procedures have been reviewed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// FAA JO 7110.65 5-7, 5-5 and 3-10.
    UnitedStates,
    /// Airservices MATS 77.3 9.5, 10.6 and 11.1.
    Australia,
}
impl Region {
    /// Unsupported regions do not inherit another authority's minima.
    #[must_use]
    pub fn for_airport(icao: &str) -> Option<Self> {
        if wake::applies(icao) {
            Some(Self::UnitedStates)
        } else if icao.len() == 4 && icao.starts_with('Y') {
            Some(Self::Australia)
        } else {
            None
        }
    }

    /// Terminal baseline plus the applicable wake pair. The caller must raise this
    /// for its modeled surveillance service; 3 NM is not a universal US minimum.
    /// Australia uses 5 NM: eligibility for a local 3 NM service is not inferred.
    /// Unknown categories are not assigned a guessed wake category.
    #[must_use]
    pub fn separation(self, leader: &Aircraft, follower: &Aircraft) -> Option<f64> {
        match self {
            Self::UnitedStates => Some(wake::approach_nm(leader.cwt?, follower.cwt?)?.max(3.0)),
            Self::Australia => {
                // MATS 10.6.4.1. Applying the Medium->Light floor to every Medium
                // is conservative where the model catalogue has no verified MTOW.
                let wake: f64 = match (leader.wake, follower.wake) {
                    ('J', 'H') => 6.0,
                    ('J', 'M') => 7.0,
                    ('J', 'L') => 8.0,
                    ('H', 'H') => 4.0,
                    ('H', 'M') => 5.0,
                    ('H', 'L') => 6.0,
                    ('M', 'L') => 5.0,
                    (a, b) if "JHML".contains(a) && "JHML".contains(b) => 0.0,
                    _ => return None,
                };
                Some(wake.max(5.0))
            }
        }
    }
}

/// Distances are along the same approach to its displaced landing threshold.
#[derive(Clone, Copy, Debug)]
pub struct Arrival {
    /// Remaining nautical miles.
    pub distance_nm: f64,
    /// Observed ground speed, knots.
    pub ground_knots: f64,
    /// Nominal final ground speed, including the available wind correction.
    pub final_knots: f64,
}

/// Planning margins, distinct from published separation minima.
#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Margins {
    /// Pilot/radio reaction allowance before changing speed.
    pub reaction_seconds: f64,
    /// Conservative acceleration/deceleration rate, knots per second.
    pub deceleration: f64,
    /// Distance where assigned speed ends and the final profile takes over.
    pub release_nm: f64,
    /// Additional space above the regional minimum.
    pub buffer_nm: f64,
    /// Threshold time interval used for planning; actual runway checks still apply.
    pub runway_seconds: f64,
}
impl Default for Margins {
    fn default() -> Self {
        Self {
            reaction_seconds: 15.0,
            deceleration: 0.5,
            release_nm: 5.0,
            buffer_nm: 0.5,
            runway_seconds: 90.0,
        }
    }
}

/// A speed solution is issued only if it solves the whole predicted encounter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    /// The current profile already leaves enough room.
    Clear,
    /// Highest feasible indicated speed that resolves the encounter.
    Reduce(f64),
    /// A protected longer path, valid delay hold or go-around is needed.
    PathRequired,
    /// Missing/invalid data cannot establish separation.
    Unknown,
}

fn valid(a: Arrival) -> bool {
    a.distance_nm.is_finite()
        && (0.0..=100.0).contains(&a.distance_nm)
        && (30.0..=400.0).contains(&a.ground_knots)
        && (30.0..=400.0).contains(&a.final_knots)
}

/// Predict closure until the leader crosses the threshold, then threshold timing.
/// Both aircraft slow for final; releasing an assigned speed cannot hide a conflict.
fn clear(
    mut follower: Arrival,
    mut leader: Arrival,
    target: f64,
    required: f64,
    m: Margins,
) -> bool {
    let mut leader_crossed = None;
    for second in 0..1800 {
        if leader.distance_nm > 0.0
            && follower.distance_nm - leader.distance_nm < required + m.buffer_nm
        {
            return false;
        }
        if follower.distance_nm <= 0.0 {
            return leader_crossed.is_some_and(|at| f64::from(second - at) >= m.runway_seconds);
        }
        if leader.distance_nm <= 0.0 && leader_crossed.is_none() {
            leader_crossed = Some(second);
        }
        let old_leader = leader.ground_knots;
        let old_follower = follower.ground_knots;
        let approach = |a: &mut Arrival, assigned: f64, react: bool| {
            let desired = if a.distance_nm <= m.release_nm {
                a.final_knots
            } else {
                assigned
            };
            if !react {
                a.ground_knots += (desired - a.ground_knots).clamp(-m.deceleration, m.deceleration);
            }
        };
        // Treat a slow leader as staying slow; never depend on its acceleration.
        let leader_target = leader.ground_knots.min(leader.final_knots);
        approach(&mut leader, leader_target, false);
        approach(
            &mut follower,
            target,
            f64::from(second) < m.reaction_seconds,
        );
        leader.distance_nm -= (old_leader + leader.ground_knots) / 7200.0;
        follower.distance_nm -= (old_follower + follower.ground_knots) / 7200.0;
    }
    false
}

/// Search at most 80 five-knot assignments, including response and deceleration time.
/// IAS-to-ground offset is measured for this aircraft; ground speed is never spoken as IAS.
#[must_use]
pub fn decide(
    follower: Arrival,
    leader: Arrival,
    indicated: f64,
    minimum: f64,
    maximum: f64,
    required: f64,
    margins: Margins,
) -> Decision {
    if !valid(follower)
        || !valid(leader)
        || leader.distance_nm >= follower.distance_nm
        || !(40.0..=400.0).contains(&indicated)
        || !(40.0..=maximum).contains(&minimum)
        || !(minimum..=400.0).contains(&maximum)
        || !(0.0..=12.0).contains(&required)
        || !(0.0..=120.0).contains(&margins.reaction_seconds)
        || !(0.1..=3.0).contains(&margins.deceleration)
        || !(5.0..=15.0).contains(&margins.release_nm)
        || !(0.0..=3.0).contains(&margins.buffer_nm)
        || !(30.0..=300.0).contains(&margins.runway_seconds)
    {
        return Decision::Unknown;
    }
    if clear(follower, leader, follower.ground_knots, required, margins) {
        return Decision::Clear;
    }
    if follower.distance_nm <= margins.release_nm || indicated < minimum {
        return Decision::PathRequired;
    }
    let offset = follower.ground_knots - indicated;
    let mut speed = ((indicated - 5.0).min(maximum) / 5.0).floor() * 5.0;
    for _ in 0..80 {
        if speed < minimum {
            break;
        }
        let ground = speed + offset;
        if (30.0..=400.0).contains(&ground) && clear(follower, leader, ground, required, margins) {
            return Decision::Reduce(speed);
        }
        speed -= 5.0;
    }
    Decision::PathRequired
}

/// Evaluate every aligned leader using one common assignment. An unresolved pair
/// prevents a speed instruction that would only solve another pair in the queue.
/// `surveillance_nm` describes the modeled service, never distance from the runway.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn assess(
    airport: &crate::airport::Airport,
    runway: &str,
    t: &crate::state::Telemetry,
    profiles: &super::profiles::Profiles,
    minimum: Option<f64>,
    maximum: Option<f64>,
    release_nm: f64,
    surveillance_nm: f64,
) -> Decision {
    if !(3.0..=10.0).contains(&surveillance_nm) {
        return Decision::Unknown;
    }
    let Some(region) = Region::for_airport(&airport.icao) else {
        return Decision::Unknown;
    };
    let Some(aircraft) = profiles.aircraft_for(&t.aircraft_icao) else {
        return Decision::Unknown;
    };
    let (Some(envelope), Some(ias), Some(targets)) =
        (&aircraft.spacing_speed, t.indicated_speed_knots, &t.traffic)
    else {
        return Decision::Unknown;
    };
    let Some(own) = super::approach_position(airport, runway, t.latitude, t.longitude) else {
        return Decision::Unknown;
    };
    let policy = &profiles.spacing;
    if t.on_ground
        || own.0 >= 0.0
        || own.1.abs() > policy.lateral_meters
        || crate::navigation::turn(t.ground_track_degrees.unwrap_or(t.heading_degrees), own.3).abs()
            > policy.alignment_degrees
    {
        return Decision::Unknown;
    }
    let offset = t.ground_speed_knots - ias;
    let follower = Arrival {
        distance_nm: -own.0 / 1852.0,
        ground_knots: t.ground_speed_knots,
        final_knots: aircraft.approach_knots + offset,
    };
    let margins = Margins {
        release_nm: release_nm.max(policy.prediction.release_nm),
        ..policy.prediction
    };
    let mut speed = ias;
    let mut pairs = Vec::new();
    let mut following = Vec::new();
    for target in targets.iter().take(126).filter(|p| !p.on_ground) {
        let Some(position) =
            super::approach_position(airport, runway, target.latitude, target.longitude)
        else {
            continue;
        };
        if position.0 >= 0.0
            || position.1.abs() > policy.lateral_meters
            || (target.altitude_feet - t.altitude_feet).abs() > policy.vertical_feet
            || crate::navigation::turn(target.track_degrees, own.3).abs() > policy.alignment_degrees
        {
            continue;
        }
        let (Some(profile), Some(ground)) = (
            profiles.aircraft_for(&target.aircraft),
            target.ground_speed_knots,
        ) else {
            return Decision::Unknown;
        };
        // Without the other aircraft's IAS/wind, use its slower nominal final speed.
        let other = Arrival {
            distance_nm: -position.0 / 1852.0,
            ground_knots: ground,
            final_knots: ground.min(profile.approach_knots),
        };
        if other.distance_nm >= follower.distance_nm {
            let Some(required) = region.separation(aircraft, profile) else {
                return Decision::Unknown;
            };
            following.push((other, required.max(surveillance_nm)));
            continue;
        }
        let Some(required) = region.separation(profile, aircraft) else {
            return Decision::Unknown;
        };
        let required = required.max(surveillance_nm);
        match decide(
            follower,
            other,
            ias,
            minimum
                .unwrap_or(envelope.minimum_knots)
                .max(envelope.minimum_knots),
            maximum
                .unwrap_or(envelope.maximum_knots)
                .min(envelope.maximum_knots),
            required,
            margins,
        ) {
            Decision::Reduce(value) => speed = speed.min(value),
            Decision::Clear => (),
            other => return other,
        }
        pairs.push((other, required));
    }
    if speed >= ias {
        return Decision::Clear;
    }
    if pairs
        .iter()
        .any(|(leader, required)| !clear(follower, *leader, speed + offset, *required, margins))
    {
        return Decision::PathRequired;
    }
    // Slowing the player cannot be the sole solution when it transfers the conflict
    // to a following aircraft that this instruction does not control.
    if following.iter().any(|(other, required)| {
        !clear(
            *other,
            Arrival {
                ground_knots: speed + offset,
                ..follower
            },
            other.ground_knots,
            *required,
            margins,
        )
    }) {
        return Decision::PathRequired;
    }
    Decision::Reduce(speed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closure_requires_a_solution_through_final_not_one_speed_step() {
        let leader = Arrival {
            distance_nm: 10.5,
            ground_knots: 150.0,
            final_knots: 140.0,
        };
        let own = Arrival {
            distance_nm: 18.0,
            ground_knots: 220.0,
            final_knots: 140.0,
        };
        let Decision::Reduce(speed) =
            decide(own, leader, 220.0, 170.0, 220.0, 3.0, Margins::default())
        else {
            panic!("expected early speed solution");
        };
        assert!((170.0..210.0).contains(&speed));
        assert_eq!(
            decide(
                Arrival {
                    distance_nm: 24.0,
                    ..own
                },
                leader,
                220.0,
                170.0,
                220.0,
                3.0,
                Margins::default()
            ),
            Decision::Clear
        );
        assert_eq!(
            decide(
                own,
                Arrival {
                    ground_knots: 70.0,
                    final_knots: 70.0,
                    ..leader
                },
                220.0,
                170.0,
                220.0,
                3.0,
                Margins::default()
            ),
            Decision::PathRequired
        );
    }
    #[test]
    fn wake_regions_final_and_unknown_observations_are_not_bypassed() {
        let heavy = super::super::profiles::aircraft("B744").unwrap();
        let light = super::super::profiles::aircraft("C172").unwrap();
        assert_eq!(Region::Australia.separation(heavy, light), Some(6.0));
        assert!(Region::UnitedStates.separation(heavy, light).unwrap() >= 6.0);
        assert_eq!(Region::for_airport("LFMN"), None);
        let own = Arrival {
            distance_nm: 4.0,
            ground_knots: 180.0,
            final_knots: 140.0,
        };
        let leader = Arrival {
            distance_nm: 2.0,
            ground_knots: 70.0,
            final_knots: 70.0,
        };
        assert_eq!(
            decide(own, leader, 180.0, 170.0, 220.0, 3.0, Margins::default()),
            Decision::PathRequired
        );
        assert_eq!(
            decide(
                Arrival {
                    ground_knots: f64::NAN,
                    ..own
                },
                leader,
                180.0,
                170.0,
                220.0,
                3.0,
                Margins::default()
            ),
            Decision::Unknown
        );
    }
}
