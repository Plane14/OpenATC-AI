//! Developer repositioning; all SDK reads and writes stay on the simulator thread.
use openatc_core::{
    airport::Airport,
    state::{Point, State},
};
use std::ffi::CString;
use xplane_sys as xp;

// X-Plane's documented quaternion order is scalar, roll, pitch, heading.
// Euler display datarefs alone are overwritten when physics resumes.
fn orientation(heading: f64, pitch: f64, roll: f64) -> [f32; 4] {
    let (s, c) = (heading.to_radians() * 0.5).sin_cos();
    let (st, ct) = (pitch.to_radians() * 0.5).sin_cos();
    let (sr, cr) = (roll.to_radians() * 0.5).sin_cos();
    [
        (c * ct * cr + s * st * sr) as f32,
        (c * ct * sr - s * st * cr) as f32,
        (c * st * cr + s * ct * sr) as f32,
        (-c * st * sr + s * ct * cr) as f32,
    ]
}

/// Let X-Plane load scenery and initialize the airborne position consistently.
/// Third-party avionics still need review before the pilot resumes the flight.
pub fn airborne(
    target: &openatc_core::debug_jump::Checkpoint,
    speed_knots: f64,
) -> Result<(), String> {
    unsafe {
        let reference = |name: &str| xp::XPLMFindDataRef(CString::new(name).unwrap().as_ptr());
        let path = reference("sim/operation/override/override_planepath");
        let tas = reference("sim/flightmodel/position/true_airspeed");
        let paused = reference("sim/time/paused");
        let pause = xp::XPLMFindCommand(CString::new("sim/operation/pause_on").unwrap().as_ptr());
        if path.is_null() || tas.is_null() || paused.is_null() || pause.is_null() {
            return Err("Native flight initialization controls unavailable.".into());
        }
        let mut overridden = 0;
        if xp::XPLMGetDatavi(path, &raw mut overridden, 0, 1) != 1 || overridden != 0 {
            return Err("Another system controls the aircraft flight path.".into());
        }
        let speed = if speed_knots == 0.0 {
            f64::from(xp::XPLMGetDataf(tas))
        } else {
            speed_knots * 0.514444
        };
        if !(20.0..=280.0).contains(&speed) {
            return Err("Choose an airborne speed (40–540 kt); zero can preserve TAS only when already flying.".into());
        }
        xp::XPLMCommandOnce(pause);
        if xp::XPLMGetDatai(paused) == 0 {
            return Err("Pause the flight before using this checkpoint.".into());
        }
        // This API performs distant scenery loads and initializes orientation,
        // velocity and running engines like X-Plane's own airborne starts.
        xp::XPLMPlaceUserAtLocation(
            target.latitude,
            target.longitude,
            (target.geometric_feet * 0.3048) as f32,
            target.track as f32,
            speed as f32,
        );
        xp::XPLMCommandOnce(pause);
    }
    Ok(())
}

/// Initialization may finish after scenery loads; do not move ATC ahead of it.
pub fn arrived(
    target: &openatc_core::debug_jump::Checkpoint,
    telemetry: &openatc_core::state::Telemetry,
) -> bool {
    telemetry.position_valid
        && telemetry.paused
        && !telemetry.on_ground
        && openatc_core::ops::distance_nm(
            target.latitude,
            target.longitude,
            telemetry.latitude,
            telemetry.longitude,
        ) < 0.5
        && (telemetry.altitude_feet - target.geometric_feet).abs() < 200.0
}

fn target(kind: &str, state: &State, airport: &Airport) -> Result<(Point, f64, f64, f64), String> {
    if !["hold", "runway"].contains(&kind) {
        return Err("Unknown debug jump target.".into());
    }
    if !state.telemetry.on_ground {
        return Err("Surface jumps require the aircraft to be on the ground.".into());
    }
    if kind == "hold" {
        let route = &state.taxi_clearance;
        if !route.approved || !route.crossing_runway.is_empty() {
            return Err("Read back a taxi clearance before jumping to its holding point.".into());
        }
        let end = *route.points.last().ok_or("No holding point available.")?;
        let before = route
            .points
            .iter()
            .rev()
            .skip(1)
            .find(|p| (p.east - end.east).hypot(p.north - end.north) > 1.0)
            .ok_or("No approach direction available.")?;
        return Ok((
            route.holding_marker_point.unwrap_or(end),
            (end.east - before.east)
                .atan2(end.north - before.north)
                .to_degrees()
                .rem_euclid(360.0),
            route.reference_latitude,
            route.reference_longitude,
        ));
    }
    if state.phase != openatc_core::state::PhaseCode::Departure {
        return Err("Obtain takeoff clearance before jumping onto the runway.".into());
    }
    if airport.icao != state.plan.departure {
        return Err("Load the departure airport on the Taxi page first.".into());
    }
    let r = airport
        .runways
        .iter()
        .find(|r| r.first_name == state.plan.runway || r.second_name == state.plan.runway)
        .ok_or("Departure runway unavailable.")?;
    let (a, b) = if r.first_name == state.plan.runway {
        (r.first, r.second)
    } else {
        (r.second, r.first)
    };
    let (de, dn) = (b.east - a.east, b.north - a.north);
    let length = de.hypot(dn);
    if length < 200.0 {
        return Err("Runway too short for this debug jump.".into());
    }
    Ok((
        Point {
            east: a.east + 50.0 * de / length,
            north: a.north + 50.0 * dn / length,
            ..a
        },
        de.atan2(dn).to_degrees().rem_euclid(360.0),
        airport.reference_latitude,
        airport.reference_longitude,
    ))
}

pub fn reposition(kind: &str, state: &State, airport: &Airport) -> Result<(), String> {
    let (point, heading, lat, lon) = target(kind, state, airport)?;
    unsafe {
        let reference = |name: &str| {
            let name = CString::new(name).expect("SDK name");
            xp::XPLMFindDataRef(name.as_ptr())
        };
        let names = [
            "sim/flightmodel/position/local_x",
            "sim/flightmodel/position/local_y",
            "sim/flightmodel/position/local_z",
        ];
        let refs = names.map(reference);
        let path = reference("sim/operation/override/override_planepath");
        let quaternion = reference("sim/flightmodel/position/q");
        if path.is_null()
            || xp::XPLMCanWriteDataRef(path) == 0
            || quaternion.is_null()
            || xp::XPLMCanWriteDataRef(quaternion) == 0
            || xp::XPLMGetDatavf(quaternion, std::ptr::null_mut(), 0, 0) != 4
            || refs
                .iter()
                .any(|r| r.is_null() || xp::XPLMCanWriteDataRef(*r) == 0)
        {
            return Err("Aircraft position controls unavailable.".into());
        }
        let mut previous = 0;
        if xp::XPLMGetDatavi(path, &raw mut previous, 0, 1) != 1 || previous != 0 {
            return Err("Another system is controlling the aircraft flight path.".into());
        }
        let probe = xp::XPLMCreateProbe(xp::XPLMProbeType::Y);
        if probe.is_null() {
            return Err("Terrain probe unavailable.".into());
        }
        let hit = |x: f64, y: f64, z: f64| {
            let mut info: xp::XPLMProbeInfo_t = std::mem::zeroed();
            info.structSize = std::mem::size_of::<xp::XPLMProbeInfo_t>() as i32;
            (xp::XPLMProbeTerrainXYZ(
                probe,
                x as f32,
                (y + 1000.0) as f32,
                z as f32,
                &raw mut info,
            ) == xp::XPLMProbeResult::HitTerrain
                && info.is_wet == 0)
                .then_some(info.locationY as f64)
        };
        let old = refs.map(|r| xp::XPLMGetDatad(r));
        let mut xyz = [0.0; 3];
        xp::XPLMWorldToLocal(
            lat + point.north / 111320.0,
            lon + point.east / (111320.0 * lat.to_radians().cos()),
            state.telemetry.altitude_feet * 0.3048,
            &raw mut xyz[0],
            &raw mut xyz[1],
            &raw mut xyz[2],
        );
        let heights = hit(old[0], old[1], old[2]).zip(hit(xyz[0], xyz[1], xyz[2]));
        xp::XPLMDestroyProbe(probe);
        let Some((old_ground, ground)) = heights else {
            return Err("No dry terrain at the debug target.".into());
        };
        xyz[1] = ground + (old[1] - old_ground).clamp(0.5, 10.0);
        let mut active = 1;
        xp::XPLMSetDatavi(path, &raw mut active, 0, 1);
        for (r, v) in refs.into_iter().zip(xyz) {
            xp::XPLMSetDatad(r, v);
        }
        let mut rotation = orientation(heading, 0.0, 0.0);
        xp::XPLMSetDatavf(quaternion, rotation.as_mut_ptr(), 0, 4);
        for (name, value) in [
            ("sim/flightmodel/position/psi", heading as f32),
            ("sim/flightmodel/position/theta", 0.0),
            ("sim/flightmodel/position/phi", 0.0),
            ("sim/flightmodel/position/local_vx", 0.0),
            ("sim/flightmodel/position/local_vy", 0.0),
            ("sim/flightmodel/position/local_vz", 0.0),
            ("sim/flightmodel/position/P", 0.0),
            ("sim/flightmodel/position/Q", 0.0),
            ("sim/flightmodel/position/R", 0.0),
            ("sim/cockpit2/controls/parking_brake_ratio", 1.0),
        ] {
            let r = reference(name);
            if !r.is_null() && xp::XPLMCanWriteDataRef(r) != 0 {
                xp::XPLMSetDataf(r, value);
            }
        }
        xp::XPLMSetDatavi(path, &raw mut previous, 0, 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_completion_waits_for_real_position_and_pause() {
        let mut telemetry = openatc_core::state::Telemetry {
            position_valid: true,
            on_ground: false,
            paused: true,
            latitude: 45.0,
            longitude: 8.0,
            altitude_feet: 5000.0,
            ..Default::default()
        };
        let target = openatc_core::debug_jump::Checkpoint {
            kind: "approach".into(),
            latitude: 45.0,
            longitude: 8.0,
            altitude_feet: 5000.0,
            geometric_feet: 5000.0,
            track: 346.0,
            next_fix: 4,
            phase: openatc_core::state::PhaseCode::Arrival,
        };
        telemetry.indicated_altitude_feet = Some(18000.0);
        assert!(arrived(&target, &telemetry));
        telemetry.paused = false;
        assert!(!arrived(&target, &telemetry));
        telemetry.paused = true;
        telemetry.on_ground = true;
        assert!(!arrived(&target, &telemetry));
        telemetry.on_ground = false;
        telemetry.latitude = 44.0;
        assert!(!arrived(&target, &telemetry));
        telemetry.latitude = 45.0;
        telemetry.altitude_feet = 18000.0;
        assert!(!arrived(&target, &telemetry));
    }
    #[test]
    fn native_orientation_sets_the_physics_quaternion() {
        assert_eq!(orientation(0.0, 0.0, 0.0), [1.0, 0.0, 0.0, 0.0]);
        let east = orientation(90.0, 0.0, 0.0);
        assert!((east[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((east[3] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        for pose in [(0.0, 3.0, 0.0), (346.0, 2.0, 0.0), (270.0, -3.0, 15.0)] {
            assert!(
                (orientation(pose.0, pose.1, pose.2)
                    .iter()
                    .map(|value| value * value)
                    .sum::<f32>()
                    - 1.0)
                    .abs()
                    < 1e-6
            );
        }
    }
    #[test]
    fn jumps_require_surface_permissions_and_use_the_safe_hold_target() {
        let mut state = State::default();
        state.telemetry.on_ground = true;
        let airport = Airport::default();
        assert!(target("hold", &state, &airport).is_err());
        assert!(target("runway", &state, &airport).is_err());
        state.taxi_clearance.approved = true;
        state.taxi_clearance.points = vec![
            Point {
                north: 0.0,
                ..Default::default()
            },
            Point {
                north: 100.0,
                ..Default::default()
            },
        ];
        state.taxi_clearance.holding_marker_point = Some(Point {
            north: 81.0,
            ..Default::default()
        });
        let (position, heading, _, _) = target("hold", &state, &airport).unwrap();
        assert_eq!(position.north, 81.0);
        assert_eq!(heading, 0.0);
        state.telemetry.on_ground = false;
        assert!(target("hold", &state, &airport).is_err());
    }
}
