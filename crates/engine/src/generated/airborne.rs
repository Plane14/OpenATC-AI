//! Solve terminal and en-route crossings together before creating movement limits.
use openatc_core::traffic::{
    motion::{Segment, Stage, Waypoint},
    trajectory::{self, Envelope, Fix},
};

pub(super) struct Section {
    pub stage: Stage,
    pub crossings: Vec<Fix>,
}

/// Published levels in later sections constrain the earlier climb and cruise too.
/// Callers must supply verified corridor protection, not a permissive placeholder.
pub(super) fn compile(
    rotation: Waypoint,
    departure_heading: f64,
    runway_heading: f64,
    sections: Vec<Section>,
    envelope: &Envelope,
    protected: impl Fn(Waypoint, Waypoint) -> bool,
) -> Result<Vec<Segment>, String> {
    if sections.len() != 4
        || sections.iter().map(|section| section.stage).ne([
            Stage::Climb,
            Stage::Cruise,
            Stage::Descent,
            Stage::Approach,
        ])
        || sections.iter().any(|section| section.crossings.is_empty())
    {
        return Err(
            "Airborne plan needs ordered departure, en-route, arrival and approach sections".into(),
        );
    }
    let count: usize = sections.iter().map(|section| section.crossings.len()).sum();
    if count > 128 {
        return Err("Combined airborne crossings exceed the planning budget".into());
    }
    let mut stages = Vec::with_capacity(count);
    let mut crossings = Vec::with_capacity(count);
    for section in sections {
        stages.extend(std::iter::repeat_n(section.stage, section.crossings.len()));
        crossings.extend(section.crossings);
    }
    let legs = trajectory::compile(
        rotation,
        departure_heading,
        &crossings,
        runway_heading,
        envelope,
        protected,
    )?;
    let segments = legs
        .into_iter()
        .zip(stages)
        .map(|(leg, stage)| Segment {
            stage,
            points: leg.points,
            knots: leg.ground_knots,
            resources: Vec::new(),
            dwell_seconds: 0.0,
        })
        .collect::<Vec<_>>();
    // The complete movement route is compiled only after a real rollout and taxi-in are attached.
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> (Waypoint, Vec<Section>, Envelope) {
        let start = Waypoint {
            latitude: 0.0,
            longitude: 0.0,
            altitude_feet: 0.0,
        };
        let sections = [
            (Stage::Climb, 0.1, 0.0, 6000.0, 6000.0),
            (Stage::Cruise, 0.2, 0.0, 6000.0, 6000.0),
            (Stage::Descent, 0.3, 0.0, 1000.0, 1000.0),
            (Stage::Approach, 0.4, 0.0, 0.0, 0.0),
        ]
        .into_iter()
        .map(
            |(stage, latitude, minimum_feet, maximum_feet, preferred_feet)| Section {
                stage,
                crossings: vec![Fix {
                    coordinate: [latitude, 0.0],
                    arrival_heading: Some(0.0),
                    minimum_feet,
                    maximum_feet,
                    preferred_feet,
                    ground_knots: 120.0,
                    required_turn: None,
                }],
            },
        )
        .collect();
        (
            start,
            sections,
            Envelope {
                bank_degrees: 20.0,
                turn_rate_degrees_sec: 3.0,
                climb_fpm: 2000.0,
                descent_fpm: 1000.0,
                spacing_meters: 250.0,
                maximum_detour_meters: 100.0,
            },
        )
    }

    #[test]
    fn arrival_ceiling_shapes_the_preceding_enroute_section() {
        let (start, sections, envelope) = inputs();
        let segments = compile(start, 0.0, 0.0, sections, &envelope, |a, b| {
            a.longitude.abs() < 0.001 && b.longitude.abs() < 0.001
        })
        .unwrap();
        assert_eq!(segments.len(), 4);
        let touchdown = *segments.last().unwrap().points.last().unwrap();
        let vacated = Waypoint {
            latitude: touchdown.latitude + 0.01,
            ..touchdown
        };
        let parked = Waypoint {
            longitude: touchdown.longitude + 0.01,
            ..vacated
        };
        let mut complete = segments.clone();
        complete.extend([
            Segment {
                stage: Stage::Rollout,
                points: vec![touchdown, vacated],
                knots: 40.0,
                resources: vec!["RUNWAY/TEST/01/19".into()],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::TaxiIn,
                points: vec![vacated, parked],
                knots: 5.0,
                resources: vec![],
                dwell_seconds: 0.0,
            },
            Segment {
                stage: Stage::Turnaround,
                points: vec![parked],
                knots: 0.0,
                resources: vec![],
                dwell_seconds: 120.0,
            },
        ]);
        openatc_core::traffic::motion::Route::compile(
            complete,
            envelope.climb_fpm,
            envelope.descent_fpm,
        )
        .unwrap();

        let cruise_end = segments[1].points.last().unwrap().altitude_feet;
        assert!(
            cruise_end < 6000.0,
            "Cruise retained an unreachable later ceiling"
        );
        assert!(segments[2].points.last().unwrap().altitude_feet <= 1000.0);
        assert_eq!(segments[3].points.last().unwrap().altitude_feet, 0.0);
        assert!(
            segments
                .windows(2)
                .all(|pair| pair[0].points.last() == pair[1].points.first())
        );
    }

    #[test]
    fn unprotected_or_incomplete_flights_do_not_create_movement() {
        let (start, sections, envelope) = inputs();
        assert!(
            compile(start, 0.0, 0.0, sections, &envelope, |_, b| b.latitude
                < 0.25)
            .is_err()
        );
        let (start, mut sections, envelope) = inputs();
        sections.pop();
        assert!(compile(start, 0.0, 0.0, sections, &envelope, |_, _| true).is_err());
    }
}
