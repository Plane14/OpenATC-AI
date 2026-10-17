//! Nominal pressure-reference and wind conversion for generated positions.
//! The planner still checks the resulting geometric path against terrain and restrictions.

const FEET: f64 = 0.3048;
const RADIUS: f64 = 6_356_766.0;
const GAS: f64 = 287.05287;
const GRAVITY: f64 = 9.80665;
const LAPSE: f64 = 0.0065;

/// A dry two-layer atmosphere calibrated to an airport observation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    pressure_hpa: f64,
    sea_kelvin: f64,
}
impl Atmosphere {
    /// Extrapolate the surface temperature to sea level with the nominal lapse rate.
    pub fn surface(
        pressure_hpa: f64,
        temperature_c: f64,
        elevation_feet: f64,
    ) -> Result<Self, String> {
        if !(800.0..=1100.0).contains(&pressure_hpa)
            || !(-60.0..=60.0).contains(&temperature_c)
            || !(-2000.0..=20000.0).contains(&elevation_feet)
        {
            return Err("Airport atmosphere is outside the nominal envelope".into());
        }
        let height = geopotential(elevation_feet);
        let sea_kelvin = temperature_c + 273.15 + LAPSE * height;
        if !(200.0..=340.0).contains(&sea_kelvin) {
            return Err("Airport temperature cannot support the nominal atmosphere".into());
        }
        Ok(Self {
            pressure_hpa,
            sea_kelvin,
        })
    }
    /// Convert a QNH altitude or standard-pressure flight level into a physical position.
    pub fn geometric(&self, level_feet: f64, standard: bool) -> Result<f64, String> {
        if !(-2000.0..=60000.0).contains(&level_feet) {
            return Err("Reported traffic level is outside the supported envelope".into());
        }
        let pressure = pressure(
            level_feet * FEET,
            if standard { 1013.25 } else { self.pressure_hpa },
            288.15,
        );
        let height = height(pressure, self.pressure_hpa, self.sea_kelvin);
        Ok(RADIUS * height / (RADIUS - height) / FEET)
    }
    /// Convert the rendered height back to the same pressure reference used by the clearance.
    pub fn reported(&self, geometric_feet: f64, standard: bool) -> Result<f64, String> {
        if !(-3000.0..=65000.0).contains(&geometric_feet) {
            return Err("Geometric traffic height is outside the supported envelope".into());
        }
        let pressure = pressure(
            geopotential(geometric_feet),
            self.pressure_hpa,
            self.sea_kelvin,
        );
        Ok(height(
            pressure,
            if standard { 1013.25 } else { self.pressure_hpa },
            288.15,
        ) / FEET)
    }
    /// Nominal calibrated/indicated speed to TAS; compressibility is not modelled here.
    pub fn true_knots(&self, indicated_knots: f64, geometric_feet: f64) -> Result<f64, String> {
        if !(35.0..=350.0).contains(&indicated_knots)
            || !(-3000.0..=65000.0).contains(&geometric_feet)
        {
            return Err("Traffic airspeed is outside the nominal envelope".into());
        }
        let h = geopotential(geometric_feet);
        let temperature = self.sea_kelvin - LAPSE * h.min(11000.0);
        let density = pressure(h, self.pressure_hpa, self.sea_kelvin) * 100.0 / (GAS * temperature);
        Ok(indicated_knots * (1.225 / density).sqrt())
    }
}
fn geopotential(feet: f64) -> f64 {
    let meters = feet * FEET;
    RADIUS * meters / (RADIUS + meters)
}
fn pressure(height: f64, sea_hpa: f64, sea_kelvin: f64) -> f64 {
    let tropopause = sea_kelvin - LAPSE * 11000.0;
    let below = sea_hpa
        * ((sea_kelvin - LAPSE * height.min(11000.0)) / sea_kelvin).powf(GRAVITY / (GAS * LAPSE));
    if height <= 11000.0 {
        below
    } else {
        below * (-GRAVITY * (height - 11000.0) / (GAS * tropopause)).exp()
    }
}
fn height(pressure: f64, sea_hpa: f64, sea_kelvin: f64) -> f64 {
    let tropopause = sea_kelvin - LAPSE * 11000.0;
    let p11 = sea_hpa * (tropopause / sea_kelvin).powf(GRAVITY / (GAS * LAPSE));
    if pressure >= p11 {
        sea_kelvin / LAPSE * (1.0 - (pressure / sea_hpa).powf(GAS * LAPSE / GRAVITY))
    } else {
        11000.0 - GAS * tropopause / GRAVITY * (pressure / p11).ln()
    }
}

/// Ground speed and the heading offset needed to maintain the planned track.
pub fn wind(
    true_knots: f64,
    track: f64,
    wind_from: f64,
    wind_knots: f64,
) -> Result<(f64, f64), String> {
    if !(35.0..=600.0).contains(&true_knots)
        || !(0.0..=200.0).contains(&wind_knots)
        || !track.is_finite()
        || !wind_from.is_finite()
    {
        return Err("Invalid traffic wind triangle".into());
    }
    let angle = (wind_from - track).to_radians();
    let cross = -wind_knots * angle.sin();
    if cross.abs() >= true_knots {
        return Err("Wind exceeds the aircraft's track-keeping speed".into());
    }
    let ground = (true_knots.powi(2) - cross.powi(2)).sqrt() - wind_knots * angle.cos();
    if !(5.0..=600.0).contains(&ground) {
        return Err("Wind leaves no supported forward ground speed".into());
    }
    Ok((ground, (-cross / true_knots).asin().to_degrees()))
}

/// Ground speed and true track when a procedure prescribes heading rather than track.
pub fn heading_wind(
    true_knots: f64,
    heading: f64,
    wind_from: f64,
    wind_knots: f64,
) -> Result<(f64, f64), String> {
    if !(35.0..=600.0).contains(&true_knots)
        || !(0.0..=200.0).contains(&wind_knots)
        || !heading.is_finite()
        || !wind_from.is_finite()
    {
        return Err("Invalid traffic heading wind triangle".into());
    }
    let angle = heading.to_radians();
    let wind = wind_from.to_radians();
    let east = true_knots * angle.sin() - wind_knots * wind.sin();
    let north = true_knots * angle.cos() - wind_knots * wind.cos();
    let speed = east.hypot(north);
    let forward = east * angle.sin() + north * angle.cos();
    if !(5.0..=600.0).contains(&speed) || forward < 5.0 {
        return Err("Wind leaves no supported forward heading movement".into());
    }
    Ok((speed, east.atan2(north).to_degrees().rem_euclid(360.0)))
}

/// True nose heading from the route's ground velocity and its retained planning wind.
pub fn heading_from_ground(
    ground_knots: f64,
    track: f64,
    wind_from: f64,
    wind_knots: f64,
) -> Result<f64, String> {
    if !(5.0..=600.0).contains(&ground_knots)
        || !(0.0..=200.0).contains(&wind_knots)
        || !track.is_finite()
        || !wind_from.is_finite()
    {
        return Err("Invalid ground velocity or planning wind".into());
    }
    let east = ground_knots * track.to_radians().sin() + wind_knots * wind_from.to_radians().sin();
    let north = ground_knots * track.to_radians().cos() + wind_knots * wind_from.to_radians().cos();
    if !(35.0..=600.0).contains(&east.hypot(north)) {
        return Err("Ground velocity has no supported airspeed in this wind".into());
    }
    Ok(east.atan2(north).to_degrees().rem_euclid(360.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prescribed_heading_keeps_drift_instead_of_crabbing_to_a_track() {
        for heading in [0.0, 90.0, 180.0, 270.0] {
            for wind_from in [0.0, 80.0, 190.0, 270.0] {
                let (ground, track) = heading_wind(150.0, heading, wind_from, 30.0).unwrap();
                let (checked, crab) = wind(150.0, track, wind_from, 30.0).unwrap();
                assert!((ground - checked).abs() < 1e-9);
                assert!(crate::navigation::turn(heading, track + crab).abs() < 1e-9);
                let nose = heading_from_ground(ground, track, wind_from, 30.0).unwrap();
                assert!(crate::navigation::turn(heading, nose).abs() < 1e-9);
            }
        }
        assert!(heading_wind(100.0, 0.0, 0.0, 150.0).is_err());
        assert!(heading_wind(100.0, f64::NAN, 0.0, 10.0).is_err());
        assert!(heading_wind(100.0, 0.0, f64::NAN, 10.0).is_err());
    }

    #[test]
    fn altitude_references_round_trip_through_both_layers() {
        for (pressure, temperature) in [(1013.25, 15.0), (1023.0, 25.0), (990.0, -10.0)] {
            let air = Atmosphere::surface(pressure, temperature, 0.0).unwrap();
            for standard in [true, false] {
                for level in [0.0, 5000.0, 18000.0, 35000.0, 41000.0, 55000.0] {
                    let geometric = air.geometric(level, standard).unwrap();
                    assert!((air.reported(geometric, standard).unwrap() - level).abs() < 0.001);
                }
            }
        }
        let hot = Atmosphere::surface(1023.0, 25.0, 0.0).unwrap();
        assert!(hot.geometric(18000.0, true).unwrap() > hot.geometric(18000.0, false).unwrap());
        assert!(hot.true_knots(200.0, 18000.0).unwrap() > 200.0);
    }
    #[test]
    fn crosswind_changes_heading_and_headwind_changes_ground_speed() {
        assert_eq!(wind(100.0, 0.0, 0.0, 10.0).unwrap(), (90.0, 0.0));
        let (speed, crab) = wind(100.0, 90.0, 0.0, 20.0).unwrap();
        assert!(speed < 100.0 && crab < 0.0);
        assert!(wind(100.0, 0.0, 0.0, 150.0).is_err());
    }
}
