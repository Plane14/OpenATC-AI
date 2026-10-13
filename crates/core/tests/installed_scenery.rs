//! Checks against the installed airport scenery used for simulator testing.
use openatc_core::{state::Telemetry, stations};

#[test]
#[ignore = "requires an installed X-Plane scenery directory"]
fn los_angeles_surface_ownership_and_stations() {
    let root = std::env::var("OPENATC_TEST_XPLANE_ROOT").expect("Set OPENATC_TEST_XPLANE_ROOT");
    let airports = stations::map_airports(&root).unwrap();
    let index = stations::load_index(&root).unwrap();
    for (latitude, longitude) in [
        (33.938282, -118.383087),
        (33.940738, -118.383234),
        (33.941596, -118.382634),
    ] {
        let telemetry = Telemetry {
            latitude,
            longitude,
            on_ground: true,
            position_valid: true,
            ..Default::default()
        };
        assert_eq!(
            stations::occupied_airport(&airports, &telemetry).map(|a| a.id.as_str()),
            Some("KLAX")
        );
        let nearby = stations::nearby(&index, &telemetry);
        for role in ["Clearance", "Ground", "Tower"] {
            assert!(
                nearby
                    .iter()
                    .any(|s| s.airport == "KLAX" && s.service == role && s.receivable),
                "Missing {role}"
            );
        }
    }
}
