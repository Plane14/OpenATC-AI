//! Airport observations retained with their age for generated route planning.
use super::*;
use openatc_core::traffic::air::Atmosphere;

#[derive(Clone)]
pub(super) struct Environment {
    pub atmosphere: Atmosphere,
    pub variation: f64,
    pub wind_from: f64,
    pub wind_knots: f64,
    observed: Instant,
}
impl Environment {
    pub fn from_sample(
        airport: &Airport,
        sample: &(serde_json::Value, Instant),
    ) -> Result<Self, String> {
        let (value, observed) = sample;
        let number = |key: &str| {
            value[key]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("Missing airport planning weather field {key}"))
        };
        if value["airport"].as_str() != Some(airport.icao.as_str())
            || (number("latitude")? - airport.reference_latitude).abs() > 0.001
            || (number("longitude")? - airport.reference_longitude).abs() > 0.001
            || (number("sampleAltitudeFeet")? - airport.elevation_feet).abs() > 50.0
        {
            return Err("Planning weather does not match the airport surface".into());
        }
        let environment = Self {
            atmosphere: Atmosphere::surface(
                number("pressureHpa")?,
                number("temperatureC")?,
                airport.elevation_feet,
            )?,
            variation: number("magneticVariationDegrees")?,
            wind_from: number("windDegrees")?,
            wind_knots: number("windKnots")?,
            observed: *observed,
        };
        if !(-180.0..=180.0).contains(&environment.variation)
            || !(0.0..=360.0).contains(&environment.wind_from)
            || !(0.0..=400.0).contains(&environment.wind_knots)
            || !environment.fresh()
        {
            return Err("Airport planning weather is stale or outside its envelope".into());
        }
        Ok(environment)
    }
    pub fn fresh(&self) -> bool {
        self.observed.elapsed() < Duration::from_secs(45)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planner_requires_current_weather_at_the_correct_airport() {
        let airport = Airport {
            icao: "TEST".into(),
            reference_latitude: 43.0,
            reference_longitude: 7.0,
            elevation_feet: 25.0,
            ..Default::default()
        };
        let mut sample = (
            serde_json::json!({"airport":"TEST","latitude":43.0,"longitude":7.0,
            "sampleAltitudeFeet":25.0,"pressureHpa":1023.0,"temperatureC":20.0,
            "magneticVariationDegrees":2.0,"windDegrees":220.0,"windKnots":12.0}),
            Instant::now(),
        );
        let weather = Environment::from_sample(&airport, &sample).unwrap();
        assert_eq!(weather.wind_from, 220.0);
        assert_eq!(weather.variation, 2.0);
        let height = weather.atmosphere.geometric(12000.0, true).unwrap();
        assert!((weather.atmosphere.reported(height, true).unwrap() - 12000.0).abs() < 0.01);
        sample.0["latitude"] = serde_json::json!(44.0);
        assert!(Environment::from_sample(&airport, &sample).is_err());
        sample.0["latitude"] = serde_json::json!(43.0);
        sample.1 = Instant::now() - Duration::from_secs(46);
        assert!(Environment::from_sample(&airport, &sample).is_err());
        sample.1 = Instant::now();
        sample
            .0
            .as_object_mut()
            .unwrap()
            .remove("magneticVariationDegrees");
        assert!(Environment::from_sample(&airport, &sample).is_err());
    }
}
