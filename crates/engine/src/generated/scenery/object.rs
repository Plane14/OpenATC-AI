//! Static OBJ8 geometry used by the worker's obstacle check.
use std::{io::Read, path::Path};

#[derive(Clone, Copy, Debug)]
pub(super) struct Bounds {
    pub low: [f64; 3],
    pub high: [f64; 3],
}

pub(super) fn load(path: &Path) -> Result<Vec<Bounds>, String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(32 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > 32 * 1024 * 1024 {
        return Err("Scenery object exceeds its input budget".into());
    }
    parse(&text)
}

fn parse(text: &str) -> Result<Vec<Bounds>, String> {
    let mut lines = text
        .lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty());
    if !matches!(lines.next(), Some("A" | "I"))
        || lines.next() != Some("800")
        || lines.next() != Some("OBJ")
    {
        return Err("Scenery obstacle requires OBJ8 geometry".into());
    }
    let mut vertices = Vec::<[f64; 3]>::new();
    let mut line_vertices = Vec::<[f64; 3]>::new();
    let mut indices = Vec::<usize>::new();
    let mut bounds = Vec::new();
    let mut counts = None;
    for line in lines {
        let mut fields = line.split_whitespace();
        let command = fields.next().unwrap();
        if command.starts_with("ANIM_") || matches!(command, "GLOBAL_tilt" | "SLOPE_LIMIT") {
            return Err("Scenery object has unresolved animation or terrain alignment".into());
        }
        let mut number = || {
            fields
                .next()
                .ok_or("Truncated OBJ8 field")?
                .parse::<usize>()
                .map_err(|_| "Invalid OBJ8 integer")
        };
        match command {
            "POINT_COUNTS" => {
                if counts.is_some() {
                    return Err("Duplicated OBJ8 point counts".into());
                }
                counts = Some([number()?, number()?, number()?, number()?]);
                if counts.unwrap().iter().any(|count| *count > 1_000_000) {
                    return Err("OBJ8 geometry exceeds its count budget".into());
                }
            }
            "VT" | "VLINE" => {
                let values = line
                    .split_whitespace()
                    .skip(1)
                    .map(|value| value.parse::<f64>())
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| "Invalid OBJ8 coordinate")?;
                let expected = if command == "VT" { 8 } else { 6 };
                if values.len() != expected
                    || values.iter().any(|value| !value.is_finite())
                    || values[..3].iter().any(|value| value.abs() > 100_000.0)
                {
                    return Err("OBJ8 vertex exceeds its shape or coordinate budget".into());
                }
                let target = if command == "VT" {
                    &mut vertices
                } else {
                    &mut line_vertices
                };
                if target.len() >= 1_000_000 {
                    return Err("OBJ8 vertex budget exceeded".into());
                }
                target.push(values[..3].try_into().unwrap());
            }
            "IDX" | "IDX10" => {
                for _ in 0..if command == "IDX" { 1 } else { 10 } {
                    if indices.len() >= 1_000_000 {
                        return Err("OBJ8 index budget exceeded".into());
                    }
                    indices.push(number()?);
                }
                if fields.next().is_some() {
                    return Err("Unexpected OBJ8 index fields".into());
                }
            }
            "TRIS" | "LINES" => {
                let start = number()?;
                let count = number()?;
                let width = if command == "TRIS" { 3 } else { 2 };
                if count % width != 0 {
                    return Err("Incomplete OBJ8 primitive".into());
                }
                let selected = indices
                    .get(start..start.checked_add(count).ok_or("OBJ8 range overflow")?)
                    .ok_or("OBJ8 primitive index range is missing")?;
                let table = if command == "TRIS" {
                    &vertices
                } else {
                    &line_vertices
                };
                for primitive in selected.chunks_exact(width) {
                    if bounds.len() >= 333_334 {
                        return Err("OBJ8 primitive budget exceeded".into());
                    }
                    let mut low = [f64::INFINITY; 3];
                    let mut high = [f64::NEG_INFINITY; 3];
                    for index in primitive {
                        let point = table
                            .get(*index)
                            .ok_or("OBJ8 primitive vertex is missing")?;
                        for axis in 0..3 {
                            low[axis] = low[axis].min(point[axis]);
                            high[axis] = high[axis].max(point[axis]);
                        }
                    }
                    bounds.push(Bounds { low, high });
                }
            }
            _ if command.starts_with("ATTR_")
                || command.starts_with("GLOBAL_")
                || command.starts_with("TEXTURE")
                || command.starts_with("LIGHT_") => {}
            "VLIGHT" | "LIGHTS" | "COCKPIT_REGION" | "DECAL_LIB" | "LOAD_CENTER"
            | "LAYER_GROUP" | "IF" | "ELSE" | "ENDIF" => {}
            _ => return Err(format!("Unresolved OBJ8 directive {command}")),
        }
    }
    let counts = counts.ok_or("OBJ8 point counts are missing")?;
    if vertices.len() != counts[0] || line_vertices.len() != counts[1] || indices.len() != counts[3]
    {
        return Err("OBJ8 tables do not match declared counts".into());
    }
    Ok(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;
    const OBJECT: &str = "I\n800\nOBJ\nPOINT_COUNTS 4 0 0 3\nVT 0 0 0 0 1 0 0 0\nVT 2 12 0 0 1 0 0 0\nVT 0 0 3 0 1 0 0 0\nVT 900 900 900 0 1 0 0 0\nIDX 0\nIDX 1\nIDX 2\nTRIS 0 3\n";

    #[test]
    fn static_obstacles_use_drawn_primitives_and_reject_unresolved_transforms() {
        let geometry = parse(OBJECT).unwrap();
        assert_eq!(geometry.len(), 1);
        assert_eq!(geometry[0].low, [0.0; 3]);
        assert_eq!(geometry[0].high, [2.0, 12.0, 3.0]);
        for extra in [
            "ANIM_rotate 0 1 0 0 90 0 1 sim/test",
            "GLOBAL_tilt",
            "SLOPE_LIMIT -1 1 -1 1",
            "UNRECOGNISED_GEOMETRY",
        ] {
            assert!(parse(&format!("{OBJECT}{extra}\n")).is_err());
        }
        for invalid in [
            OBJECT.replace("IDX 2", "IDX 9"),
            OBJECT.replace("TRIS 0 3", "TRIS 0 6"),
            OBJECT.replace("VT 2 12", "VT 2 NaN"),
            OBJECT.replace("4 0 0 3", "3 0 0 3"),
        ] {
            assert!(parse(&invalid).is_err());
        }
    }
}
