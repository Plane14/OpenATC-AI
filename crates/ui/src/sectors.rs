//! Debug view of the same published radio volumes used by controller selection.
use crate::{interface::Interface, widgets};
use openatc_core::state::State;

// Both kinds of label share space; airport names must not cover controller names.
fn reserve_label(
    labels: &mut Vec<[f32; 4]>,
    point: [f32; 2],
    extent: [f32; 2],
    start: [f32; 2],
    size: [f32; 2],
) -> bool {
    let bounds = [
        point[0] - 4.0,
        point[1] - 3.0,
        point[0] + extent[0] + 4.0,
        point[1] + extent[1] + 3.0,
    ];
    if bounds[0] < start[0]
        || bounds[1] < start[1]
        || bounds[2] > start[0] + size[0]
        || bounds[3] > start[1] + size[1]
        || labels
            .iter()
            .any(|b| bounds[0] < b[2] && bounds[2] > b[0] && bounds[1] < b[3] && bounds[3] > b[1])
    {
        return false;
    }
    labels.push(bounds);
    true
}

pub fn page(ui: &imgui::Ui, interface: &mut Interface, state: &State) {
    if ui.button("Follow aircraft") {
        interface.radio_map_center = None;
    }
    let position = format!(
        "{:.0} ft MSL / COM1 {:.3}",
        state.telemetry.altitude_feet,
        f64::from(state.telemetry.com1_khz) / 1000.
    );
    widgets::same_line_if_fits(ui, ui.calc_text_size(&position)[0]);
    ui.text(position);
    ui.checkbox("Other altitude bands", &mut interface.map_all_altitudes);
    widgets::same_line_if_fits(
        ui,
        ui.calc_text_size("Airports")[0]
            + ui.frame_height()
            + ui.clone_style().item_inner_spacing[0],
    );
    ui.checkbox("Airports", &mut interface.map_airports_visible);
    widgets::same_line_if_fits(
        ui,
        ui.calc_text_size("Heliports")[0]
            + ui.frame_height()
            + ui.clone_style().item_inner_spacing[0],
    );
    ui.checkbox("Heliports", &mut interface.map_heliports);
    ui.text_wrapped("Airports and controller coverage from installed scenery. Wheel to zoom; drag to pan; hover for controller names and limits. Follow aircraft restores the moving map.");
    widgets::muted_wrapped(ui, &interface.map_notice);
    if state.recommended_frequency_khz > 0 {
        ui.text_colored(
            [0.95, 0.75, 0.3, 1.0],
            format!(
                "Assigned frequency {:.3} MHz",
                f64::from(state.recommended_frequency_khz) / 1000.
            ),
        );
    }
    let start = ui.cursor_screen_pos();
    let available = ui.content_region_avail();
    let size = [available[0].max(100.), available[1].max(180.)];
    ui.invisible_button("##radio-sector-map", size);
    let hovered = ui.is_item_hovered();
    if hovered {
        interface.radio_map_range =
            (interface.radio_map_range * 1.2_f32.powf(-ui.io().mouse_wheel)).clamp(2., 1500.);
    }
    let t = &state.telemetry;
    let mut center = interface
        .radio_map_center
        .unwrap_or([t.latitude, t.longitude]);
    let scale = f64::from(size[0].min(size[1])) / (2. * f64::from(interface.radio_map_range));
    let cosine = center[0].to_radians().cos().abs().max(0.05);
    if hovered && ui.is_item_active() && ui.is_mouse_dragging(imgui::MouseButton::Left) {
        center[0] += f64::from(ui.io().mouse_delta[1]) / (scale * 60.);
        center[1] -= f64::from(ui.io().mouse_delta[0]) / (scale * 60. * cosine);
        interface.radio_map_center = Some(center);
    }
    let middle = [start[0] + size[0] * 0.5, start[1] + size[1] * 0.5];
    let project = |p: [f64; 2]| {
        let lon = (p[1] - center[1] + 180.).rem_euclid(360.) - 180.;
        [
            middle[0] + (lon * cosine * 60. * scale) as f32,
            middle[1] - ((p[0] - center[0]) * 60. * scale) as f32,
        ]
    };
    let draw = ui.get_window_draw_list();
    draw.add_rect(
        start,
        [start[0] + size[0], start[1] + size[1]],
        [0.07, 0.10, 0.13, 1.],
    )
    .filled(true)
    .build();
    let mut outlines = 0;
    draw.with_clip_rect(start, [start[0] + size[0], start[1] + size[1]], || {
        let grid = (f64::from(interface.radio_map_range) / 4.).max(1.);
        for n in -4..=4 {
            let offset = (f64::from(n) * grid * scale) as f32;
            draw.add_line(
                [middle[0] + offset, start[1]],
                [middle[0] + offset, start[1] + size[1]],
                [0.35, 0.5, 0.6, 0.12],
            )
            .build();
            draw.add_line(
                [start[0], middle[1] + offset],
                [start[0] + size[0], middle[1] + offset],
                [0.35, 0.5, 0.6, 0.12],
            )
            .build();
        }
        let mut labels = vec![[start[0], start[1], start[0] + size[0], start[1] + 40.0]];
        let mut controller_labels = Vec::new();
        let mut stations: Vec<_> = interface.map_stations.iter().collect();
        stations.sort_by_key(|s| s.khz != t.com1_khz);
        let mut drawn = Vec::<&openatc_core::stations::Coverage>::new();
        for station in stations {
            for volume in &station.coverage {
                if volume.points.len() < 3
                    || (!interface.map_all_altitudes
                        && (t.altitude_feet < volume.floor || t.altitude_feet > volume.ceiling))
                {
                    continue;
                }
                if drawn.iter().any(|v| {
                    v.floor == volume.floor
                        && v.ceiling == volume.ceiling
                        && v.points == volume.points
                }) {
                    continue;
                }
                drawn.push(volume);
                outlines += 1;
                let active = station.khz == t.com1_khz && volume.contains(t);
                let at_altitude =
                    t.altitude_feet >= volume.floor && t.altitude_feet <= volume.ceiling;
                let color = if active {
                    [0.25, 0.90, 0.72, 0.95]
                } else if !at_altitude {
                    [0.55, 0.55, 0.65, 0.25]
                } else if station.service == "Center" {
                    [0.85, 0.6, 0.95, 0.7]
                } else {
                    [0.35, 0.65, 0.85, 0.75]
                };
                let mut previous = project(volume.points[volume.points.len() - 1]);
                for point in &volume.points {
                    let next = project(*point);
                    let outside = (previous[0] < start[0] && next[0] < start[0])
                        || (previous[1] < start[1] && next[1] < start[1])
                        || (previous[0] > start[0] + size[0] && next[0] > start[0] + size[0])
                        || (previous[1] > start[1] + size[1] && next[1] > start[1] + size[1]);
                    if !outside {
                        draw.add_line(previous, next, color)
                            .thickness(if active { 3. } else { 1.5 })
                            .build();
                    }
                    previous = next;
                }
                let center_point = volume
                    .points
                    .iter()
                    .fold([0., 0.], |a, p| [a[0] + p[0], a[1] + p[1]]);
                let label = project([
                    center_point[0] / volume.points.len() as f64,
                    center_point[1] / volume.points.len() as f64,
                ]);
                let title = format!("{} · {:.3}", station.name, f64::from(station.khz) / 1000.);
                let limits = format!(
                    "{} / {:.0}–{:.0} ft",
                    station.service, volume.floor, volume.ceiling
                );
                let extent = [
                    ui.calc_text_size(&title)[0].max(ui.calc_text_size(&limits)[0]),
                    ui.text_line_height() * 2.0,
                ];
                if reserve_label(&mut labels, label, extent, start, size) {
                    controller_labels.push((label, extent, color, title, limits));
                }
            }
        }
        let mut airports: Vec<_> = interface
            .map_airports
            .iter()
            .filter(|a| interface.map_airports_visible && (interface.map_heliports || !a.heliport))
            .collect();
        airports.sort_by_key(|a| {
            (
                a.id != state.plan.departure && a.id != state.plan.destination,
                !a.id.chars().all(|c| c.is_ascii_uppercase()),
            )
        });
        for airport in airports {
            let p = project([airport.latitude, airport.longitude]);
            if p[0] < start[0]
                || p[1] < start[1]
                || p[0] > start[0] + size[0]
                || p[1] > start[1] + size[1]
            {
                continue;
            }
            draw.add_circle(p, 3.5, [0.95, 0.80, 0.4, 1.])
                .filled(true)
                .build();
            let label = [p[0] + 6.0, p[1] - ui.text_line_height() * 0.5];
            let extent = ui.calc_text_size(&airport.id);
            if interface.radio_map_range <= 150.
                && reserve_label(&mut labels, label, extent, start, size)
            {
                draw.add_rect(
                    [label[0] - 2.0, label[1] - 2.0],
                    [label[0] + extent[0] + 2.0, label[1] + extent[1] + 2.0],
                    [0.07, 0.10, 0.13, 0.85],
                )
                .filled(true)
                .build();
                draw.add_text(label, [0.95, 0.85, 0.6, 1.], &airport.id);
            }
        }
        for (label, extent, color, title, limits) in controller_labels {
            draw.add_rect(
                [label[0] - 3.0, label[1] - 2.0],
                [label[0] + extent[0] + 3.0, label[1] + extent[1] + 2.0],
                [0.07, 0.10, 0.13, 0.94],
            )
            .filled(true)
            .rounding(3.0)
            .build();
            draw.add_text(label, color, title);
            draw.add_text([label[0], label[1] + ui.text_line_height()], color, limits);
        }
        let aircraft = project([t.latitude, t.longitude]);
        let angle = t.heading_degrees.to_radians();
        let point = |x: f64, y: f64| {
            [
                aircraft[0] + (x * angle.cos() - y * angle.sin()) as f32,
                aircraft[1] + (x * angle.sin() + y * angle.cos()) as f32,
            ]
        };
        draw.add_triangle(
            point(0., -12.),
            point(-7., 8.),
            point(7., 8.),
            [0.95, 0.97, 1., 1.],
        )
        .filled(true)
        .build();
        draw.add_text(
            [start[0] + 12., start[1] + 12.],
            [0.8, 0.9, 1., 1.],
            format!("NORTH UP / {:.0} NM", interface.radio_map_range),
        );
        if outlines == 0 {
            draw.add_text(
                [start[0] + 12., start[1] + 36.],
                [1., 0.75, 0.35, 1.],
                "No published sector polygons at this altitude. Boundaries are not invented.",
            );
        }
    });
    if hovered {
        let mouse = ui.io().mouse_pos;
        let lat = center[0] - f64::from(mouse[1] - middle[1]) / (scale * 60.);
        let lon = center[1] + f64::from(mouse[0] - middle[0]) / (scale * 60. * cosine);
        let probe = openatc_core::state::Telemetry {
            latitude: lat,
            longitude: lon,
            ..t.clone()
        };
        let controllers: Vec<_> = interface
            .map_stations
            .iter()
            .filter(|s| {
                s.coverage.iter().any(|v| {
                    v.contains(&probe)
                        || (interface.map_all_altitudes && v.radio_distance_nm(&probe) == 0.)
                })
            })
            .collect();
        let airports: Vec<_> = interface
            .map_airports
            .iter()
            .filter(|a| {
                if !interface.map_airports_visible || (a.heliport && !interface.map_heliports) {
                    return false;
                }
                let p = project([a.latitude, a.longitude]);
                (p[0] - mouse[0]).hypot(p[1] - mouse[1]) < 12.
            })
            .collect();
        if !controllers.is_empty() || !airports.is_empty() {
            ui.tooltip(|| {
                let _wrap = ui.push_text_wrap_pos_with_pos(ui.current_font_size() * 24.0);
                for airport in airports {
                    ui.text_wrapped(format!("{} · {}", airport.id, airport.name));
                }
                let mut groups = std::collections::BTreeMap::<_, Vec<_>>::new();
                for station in controllers {
                    groups
                        .entry((&station.name, &station.service))
                        .or_default()
                        .push(station);
                }
                for ((name, service), stations) in groups.iter().take(4) {
                    ui.separator();
                    ui.text_wrapped(format!("{name} · {service}"));
                    let mut frequencies: Vec<_> = stations.iter().map(|s| s.khz).collect();
                    frequencies.sort_unstable();
                    frequencies.dedup();
                    let listed = frequencies
                        .iter()
                        .take(8)
                        .map(|khz| format!("{:.3}", f64::from(*khz) / 1000.))
                        .collect::<Vec<_>>()
                        .join(", ");
                    ui.text_wrapped(format!("{listed} MHz"));
                    if frequencies.len() > 8 {
                        widgets::muted_wrapped(
                            ui,
                            format!("{} more published frequencies", frequencies.len() - 8),
                        );
                    }
                    let mut limits = Vec::new();
                    for station in stations {
                        for volume in &station.coverage {
                            if volume.contains(&probe)
                                || (interface.map_all_altitudes
                                    && volume.radio_distance_nm(&probe) == 0.)
                            {
                                let range =
                                    format!("{:.0}–{:.0} ft MSL", volume.floor, volume.ceiling);
                                if !limits.contains(&range) {
                                    limits.push(range);
                                }
                            }
                        }
                    }
                    widgets::muted_wrapped(ui, limits.join(" / "));
                }
                if groups.len() > 4 {
                    widgets::muted_wrapped(
                        ui,
                        format!("{} more overlapping controller groups", groups.len() - 4),
                    );
                }
            });
        }
    }
}
