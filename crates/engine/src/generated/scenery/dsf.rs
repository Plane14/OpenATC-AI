//! Bounded DSF v1 mesh decoding; unfamiliar placement commands remain errors.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
use super::{DrapedPolygon, Placement, Tile, Vertex};

/// Dense installed tiles can exceed one million polygon points. Retain them up to this bound and
/// let the tile geometry byte budget decide whether they fit.
const MAX_POLYGON_POINTS: usize = 2 * 1024 * 1024;

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .ok_or("DSF field size overflow")?;
        let result = self.bytes.get(self.at..end).ok_or("Truncated DSF record")?;
        self.at = end;
        Ok(result)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f64, String> {
        Ok(f64::from(f32::from_le_bytes(
            self.take(4)?.try_into().unwrap(),
        )))
    }
}

type Atom<'a> = ([u8; 4], &'a [u8]);

fn atoms(bytes: &[u8]) -> Result<Vec<Atom<'_>>, String> {
    let mut reader = Reader::new(bytes);
    let mut result = Vec::new();
    while reader.at < bytes.len() {
        if result.len() >= 65536 {
            return Err("DSF atom count exceeds the tile budget".into());
        }
        let id = reader.take(4)?.try_into().unwrap();
        let size = reader
            .u32()?
            .checked_sub(8)
            .ok_or("Invalid DSF atom size")?;
        result.push((id, reader.take(size as usize)?));
    }
    Ok(result)
}
fn strings(bytes: &[u8]) -> Result<Vec<&str>, String> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.last() != Some(&0) {
        return Err("DSF string table is not terminated".into());
    }
    bytes[..bytes.len() - 1]
        .split(|byte| *byte == 0)
        .map(|part| {
            if part.len() > 8192 {
                return Err("DSF string exceeds its budget".into());
            }
            std::str::from_utf8(part).map_err(|_| "DSF string is not UTF-8".into())
        })
        .collect()
}
fn only<'a>(atoms: &[([u8; 4], &'a [u8])], id: [u8; 4]) -> Result<&'a [u8], String> {
    let mut found = atoms.iter().filter(|(key, _)| *key == id);
    let result = found.next().ok_or("Required DSF atom is missing")?.1;
    if found.next().is_some() {
        return Err("Required DSF atom is duplicated".into());
    }
    Ok(result)
}

struct Pool {
    depth: usize,
    points: Vec<[f64; 4]>,
}

fn draped(names: &[&str], definition: usize) -> bool {
    names
        .get(definition)
        .and_then(|name| std::path::Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("pol")
                || extension.eq_ignore_ascii_case("lin")
                || extension.eq_ignore_ascii_case("bch")
        })
}
fn extension_is(names: &[&str], definition: usize, wanted: &str) -> bool {
    names
        .get(definition)
        .and_then(|name| std::path::Path::new(name).extension())
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(wanted))
}

/// A polygon definition is a facade when its file is a `.fac`. The DSF polygon parameter is
/// then the building height in metres rather than a texture or density value.
fn facade(names: &[&str], definition: usize) -> bool {
    extension_is(names, definition, "fac")
}

/// A forest polygon fills its area with trees. Unlike a facade its parameter is a density, so
/// the height comes from the tree definitions in the `.for` file instead.
fn forest(names: &[&str], definition: usize) -> bool {
    extension_is(names, definition, "for")
}

/// A polygon that rises above the ground instead of draping over it: a building or a forest.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct Facade {
    pub resource: std::sync::Arc<str>,
    /// Geographic footprint, outer ring first, latitude and longitude.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// Published building height in metres, taken from the polygon parameter. A forest has no
    /// height here: its trees are defined in the referenced `.for` file.
    pub height_meters: f64,
    /// Trees rather than a building; the height is resolved from the forest definition.
    pub forest: bool,
}

/// A polygon definition that repeats objects along its rings: an object string (`.str`) or an
/// autogen string (`.ags`). Its height comes from the catalogue it references.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StringPolygon {
    pub resource: std::sync::Arc<str>,
    /// Geographic footprint, latitude and longitude.
    pub rings: Vec<Vec<[f64; 2]>>,
    /// Autogen strings also place vegetation; object strings place only objects.
    pub autogen: bool,
    /// Height of the tallest placed object, set when the catalogue is resolved.
    pub height_meters: f64,
    /// Horizontal reach of a placed object, used to widen each string segment.
    pub reach_meters: f64,
    pub ground_reach_meters: Option<f64>,
}

fn string_definition(names: &[&str], definition: usize) -> Option<bool> {
    let extension = std::path::Path::new(names.get(definition)?)
        .extension()?
        .to_str()?;
    if extension.eq_ignore_ascii_case("str") {
        Some(false)
    } else if extension.eq_ignore_ascii_case("ags") || extension.eq_ignore_ascii_case("agb") {
        Some(true)
    } else {
        None
    }
}

#[allow(clippy::option_option)] // Absent, excluded, replaced.
fn filter_comment(
    comment: &[u8],
    filters: &[std::sync::Arc<str>],
) -> Result<Option<Option<std::sync::Arc<str>>>, String> {
    if comment.len() < 2 || u16::from_le_bytes(comment[..2].try_into().unwrap()) != 1 {
        return Ok(None);
    }
    let index = comment
        .get(2..6)
        .ok_or("Truncated DSF airport filter comment")?;
    let index = i32::from_le_bytes(index.try_into().unwrap());
    if index == -1 {
        return Ok(Some(None));
    }
    let index = usize::try_from(index).map_err(|_| "Invalid DSF airport filter index")?;
    Ok(Some(Some(
        filters
            .get(index)
            .ok_or("DSF airport filter is missing")?
            .clone(),
    )))
}

fn polygon_indices(reader: &mut Reader<'_>, command: u8) -> Result<(u16, Vec<Vec<u16>>), String> {
    let parameter = reader.u16()?;
    let rings = match command {
        12 => {
            let count = reader.u8()?;
            vec![
                (0..count)
                    .map(|_| reader.u16())
                    .collect::<Result<Vec<_>, _>>()?,
            ]
        }
        13 => {
            let start = reader.u16()?;
            let end = reader.u16()?;
            if end < start {
                return Err("Reversed DSF polygon range".into());
            }
            vec![(start..end).collect()]
        }
        14 => {
            let count = reader.u8()?;
            (0..count)
                .map(|_| {
                    let count = reader.u8()?;
                    (0..count)
                        .map(|_| reader.u16())
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        15 => {
            // The command stores the winding count, followed by one extra range endpoint.
            let count = reader.u8()?;
            let mut start = reader.u16()?;
            let mut rings = Vec::new();
            for _ in 0..count {
                let end = reader.u16()?;
                if end < start {
                    return Err("Reversed DSF polygon winding range".into());
                }
                rings.push((start..end).collect());
                start = end;
            }
            rings
        }
        _ => return Err("Unknown DSF polygon command".into()),
    };
    Ok((parameter, rings))
}

/// Decode one coordinate pool. Geometry pools hold 16-bit fixed point; vector networks hold
/// 32-bit fixed point and are large enough to need no junction indirection. `width` is the
/// element size in bytes and fixes both the value range and the differencing modulus.
fn pool(bytes: &[u8], scale: &[u8], width: usize) -> Result<Pool, String> {
    let mut reader = Reader::new(bytes);
    let count = reader.u32()? as usize;
    let depth = usize::from(reader.u8()?);
    // An empty pool carries no planes, so its depth is meaningless; the default scenery writes
    // such a pool. Any pool with points must still declare a plausible depth.
    let limit = if width == 2 { 65536 } else { 4 * 1024 * 1024 };
    if count > limit || depth > 32 || (depth == 0 && count != 0) || scale.len() != depth * 8 {
        return Err("DSF point pool exceeds its shape or count budget".into());
    }
    let mut points = vec![[0.0; 4]; count];
    let mut scaling = Reader::new(scale);
    let mut plane = 0;
    while plane < depth {
        let factor = scaling.f32()?;
        let offset = scaling.f32()?;
        if !factor.is_finite() || !offset.is_finite() {
            return Err("Invalid DSF coordinate scale".into());
        }
        let encoding = reader.u8()?;
        if encoding > 3 {
            return Err("Unknown DSF point encoding".into());
        }
        let mut values = Vec::with_capacity(count);
        while values.len() < count {
            if encoding & 2 == 0 {
                values.push(read_value(&mut reader, width)?);
            } else {
                let run = reader.u8()?;
                let length = usize::from(run & 127);
                if length == 0 || length > count - values.len() {
                    return Err("Invalid DSF point run".into());
                }
                if run & 128 != 0 {
                    let value = read_value(&mut reader, width)?;
                    values.resize(values.len() + length, value);
                } else {
                    for _ in 0..length {
                        values.push(read_value(&mut reader, width)?);
                    }
                }
            }
        }
        let divisor = if width == 2 { 65535.0 } else { 4_294_967_295.0 };
        let mut last = 0u32;
        for (index, value) in values.into_iter().enumerate() {
            let value = if encoding & 1 != 0 {
                if width == 2 {
                    (last + value) & 0xFFFF
                } else {
                    last.wrapping_add(value)
                }
            } else {
                value
            };
            last = value;
            if plane < 4 {
                points[index][plane] = if factor == 0.0 {
                    f64::from(value)
                } else {
                    f64::from(value) / divisor * factor + offset
                };
            }
        }
        plane += 1;
    }
    if reader.at != bytes.len() {
        return Err("Unexpected trailing DSF point data".into());
    }
    Ok(Pool { depth, points })
}

fn read_value(reader: &mut Reader<'_>, width: usize) -> Result<u32, String> {
    Ok(if width == 2 {
        u32::from(reader.u16()?)
    } else {
        reader.u32()?
    })
}

struct Raster {
    width: usize,
    height: usize,
    post: bool,
    values: Vec<f64>,
}
impl Raster {
    fn read(info: &[u8], bytes: &[u8]) -> Result<Self, String> {
        let mut reader = Reader::new(info);
        if reader.u8()? != 1 {
            return Err("Unsupported DSF raster version".into());
        }
        let size = usize::from(reader.u8()?);
        let flags = reader.u16()?;
        let width = reader.u32()? as usize;
        let height = reader.u32()? as usize;
        let scale = reader.f32()?;
        let offset = reader.f32()?;
        let count = width
            .checked_mul(height)
            .ok_or("DSF raster size overflow")?;
        if !(2..=8192).contains(&width)
            || !(2..=8192).contains(&height)
            || count > 16 * 1024 * 1024
            || !matches!(size, 1 | 2 | 4)
            || flags & !7 != 0
            || flags & 3 == 3
            || flags.trailing_zeros() >= 2 && size != 4
            || !scale.is_finite()
            || !offset.is_finite()
            || reader.at != info.len()
            || count.checked_mul(size) != Some(bytes.len())
        {
            return Err("Unsupported DSF raster shape or encoding".into());
        }
        let values = bytes
            .chunks_exact(size)
            .map(|value| {
                let raw = match (flags & 3, size) {
                    (0, 4) => f64::from(f32::from_le_bytes(value.try_into().unwrap())),
                    (1, 1) => f64::from(value[0] as i8),
                    (1, 2) => f64::from(i16::from_le_bytes(value.try_into().unwrap())),
                    (1, 4) => f64::from(i32::from_le_bytes(value.try_into().unwrap())),
                    (2, 1) => f64::from(value[0]),
                    (2, 2) => f64::from(u16::from_le_bytes(value.try_into().unwrap())),
                    (2, 4) => f64::from(u32::from_le_bytes(value.try_into().unwrap())),
                    _ => unreachable!(),
                };
                let height = raw * scale + offset;
                if !height.is_finite() || !(-1500.0..=12000.0).contains(&height) {
                    return Err("DSF elevation raster has an invalid or unknown height".into());
                }
                Ok(height)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            width,
            height,
            post: flags & 4 != 0,
            values,
        })
    }
    fn upper(&self, coordinate: [f64; 2], bounds: [f64; 4]) -> Result<f64, String> {
        let axis = |value, low, high, size: usize| {
            let t: f64 = (value - low) / (high - low);
            let x = if self.post {
                t * (size - 1) as f64
            } else {
                t * size as f64 - 0.5
            };
            let first = x.floor().clamp(0.0, (size - 1) as f64) as usize;
            [first, (first + 1).min(size - 1)]
        };
        let x = axis(coordinate[0], bounds[0], bounds[1], self.width);
        let y = axis(coordinate[1], bounds[2], bounds[3], self.height);
        x.into_iter()
            .flat_map(|x| y.into_iter().map(move |y| self.values[y * self.width + x]))
            .reduce(f64::max)
            .ok_or_else(|| "Missing DSF raster neighbors".into())
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<Tile, String> {
    if bytes.len() < 28 || &bytes[..8] != b"XPLNEDSF" || bytes[8..12] != 1u32.to_le_bytes() {
        return Err("Unsupported or truncated DSF header".into());
    }
    let top = atoms(&bytes[12..bytes.len() - 16])?;
    let header = atoms(only(&top, *b"DAEH")?)?;
    let properties = strings(only(&header, *b"PORP")?)?;
    if properties.len() % 2 != 0 {
        return Err("Invalid DSF property pairs".into());
    }
    let polygon_exclusions = properties
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| pair[0] == "sim/exclude_pol")
        .map(|pair| {
            let bounds = pair[1]
                .split(';')
                .next()
                .unwrap_or("")
                .split('/')
                .map(|value| {
                    value
                        .parse::<f64>()
                        .map_err(|_| "Invalid DSF polygon exclusion")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if bounds.len() != 4
                || bounds.iter().any(|value| !value.is_finite())
                || bounds[0] > bounds[2]
                || bounds[1] > bounds[3]
            {
                return Err("Invalid DSF polygon exclusion bounds");
            }
            Ok([bounds[0], bounds[2], bounds[1], bounds[3]])
        })
        .collect::<Result<Vec<_>, _>>()?;
    let airport_filters = properties
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| pair[0] == "sim/filter/aptid")
        .map(|pair| std::sync::Arc::<str>::from(pair[1]))
        .collect::<Vec<_>>();
    let property = |name: &str| {
        properties
            .as_chunks::<2>()
            .0
            .iter()
            .find(|pair| pair[0] == name)
            .map(|pair| pair[1])
    };
    if property("sim/planet").is_some_and(|planet| planet != "earth") {
        return Err("Traffic scenery must use the Earth coordinate reference".into());
    }
    let bound = |name| {
        property(name)
            .ok_or("Required DSF tile boundary is missing")?
            .parse::<f64>()
            .map_err(|_| "Invalid DSF tile boundary")
    };
    let bounds = [
        bound("sim/west")?,
        bound("sim/east")?,
        bound("sim/south")?,
        bound("sim/north")?,
    ];
    if bounds.iter().any(|n| !n.is_finite())
        || (bounds[1] - bounds[0] - 1.0).abs() > 1e-8
        || (bounds[3] - bounds[2] - 1.0).abs() > 1e-8
        || bounds[0] < -180.0
        || bounds[1] > 180.0
        || bounds[2] < -85.0
        || bounds[3] > 85.0
    {
        return Err("DSF tile bounds exceed the supported projection".into());
    }
    let definitions = atoms(only(&top, *b"NFED")?)?;
    let polygon_names = definitions
        .iter()
        .find(|(key, _)| key == b"YLOP")
        .map(|(_, bytes)| strings(bytes))
        .transpose()?
        .unwrap_or_default();
    if polygon_names.len() > 65536
        || polygon_names.iter().map(|name| name.len()).sum::<usize>() > 4 * 1024 * 1024
    {
        return Err("DSF polygon definitions exceed their catalogue budget".into());
    }
    let object_names = definitions
        .iter()
        .find(|(key, _)| key == b"TJBO")
        .map(|(_, bytes)| strings(bytes))
        .transpose()?
        .unwrap_or_default()
        .into_iter()
        .map(std::sync::Arc::<str>::from)
        .collect::<Vec<_>>();
    if object_names.len() > 65536
        || object_names.iter().map(|name| name.len()).sum::<usize>() > 4 * 1024 * 1024
    {
        return Err("DSF object definitions exceed their catalogue budget".into());
    }
    let raster_names = definitions
        .iter()
        .find(|(key, _)| key == b"NMED")
        .map(|(_, data)| strings(data))
        .transpose()?
        .unwrap_or_default();
    let elevation = raster_names.iter().position(|name| *name == "elevation");
    let raster = if let Some(index) = elevation {
        let rasters = atoms(only(&top, *b"SMED")?)?;
        let info = rasters
            .iter()
            .filter(|(key, _)| key == b"IMED")
            .nth(index)
            .ok_or("Missing elevation raster metadata")?
            .1;
        let data = rasters
            .iter()
            .filter(|(key, _)| key == b"DMED")
            .nth(index)
            .ok_or("Missing elevation raster pixels")?
            .1;
        Some(Raster::read(info, data)?)
    } else {
        None
    };
    let geodata = atoms(only(&top, *b"DOEG")?)?;
    let raw_pools = geodata
        .iter()
        .filter(|(key, _)| key == b"LOOP")
        .map(|(_, bytes)| *bytes)
        .collect::<Vec<_>>();
    let scales = geodata
        .iter()
        .filter(|(key, _)| key == b"LACS")
        .map(|(_, bytes)| *bytes)
        .collect::<Vec<_>>();
    if raw_pools.len() != scales.len() || raw_pools.len() > 8192 {
        return Err("DSF coordinate pools have missing scales or exceed their budget".into());
    }
    let mut pools = Vec::new();
    let mut points = 0usize;
    for (raw, scaling) in raw_pools.into_iter().zip(scales) {
        let decoded = pool(raw, scaling, 2)?;
        points += decoded.points.len();
        if points > 4 * 1024 * 1024 {
            return Err("DSF coordinate count exceeds the mesh budget".into());
        }
        pools.push(decoded);
    }
    // Vector networks use their own 32-bit pools, selected by the same pool index but resolved
    // against this list.
    let network_raw = geodata
        .iter()
        .filter(|(key, _)| key == b"23OP")
        .map(|(_, bytes)| *bytes)
        .collect::<Vec<_>>();
    let network_scales = geodata
        .iter()
        .filter(|(key, _)| key == b"23CS")
        .map(|(_, bytes)| *bytes)
        .collect::<Vec<_>>();
    if network_raw.len() != network_scales.len() || network_raw.len() > 8192 {
        return Err(
            "DSF vector coordinate pools have missing scales or exceed their budget".into(),
        );
    }
    let mut network_pools = Vec::new();
    for (raw, scaling) in network_raw.into_iter().zip(network_scales) {
        let decoded = pool(raw, scaling, 4)?;
        points += decoded.points.len();
        if points > 4 * 1024 * 1024 {
            return Err("DSF coordinate count exceeds the mesh budget".into());
        }
        network_pools.push(decoded);
    }
    let mut reader = Reader::new(only(&top, *b"SDMC")?);
    let mut current = None;
    let mut physical = false;
    let mut triangles = Vec::new();
    let mut unresolved_obstacles = false;
    let mut definition = 0usize;
    let mut objects = Vec::new();
    let mut draped_resources = std::collections::BTreeSet::new();
    let mut polygons = Vec::new();
    let mut facades = Vec::new();
    let mut strings: Vec<StringPolygon> = Vec::new();
    let mut polygon_resources = std::collections::BTreeMap::new();
    let mut polygon_points = 0usize;
    let mut airport_filter = None;
    let mut junction_offset = 0u32;
    let mut networks: Vec<Vec<[f64; 3]>> = Vec::new();
    let mut network_points = 0usize;
    while reader.at < reader.bytes.len() {
        let command = reader.u8()?;
        match command {
            1 => {
                current = Some(usize::from(reader.u16()?));
            }
            2 => {
                // Retained: vector commands below 11 add this to their indices.
                junction_offset = reader.u32()?;
            }
            3 => definition = usize::from(reader.u8()?),
            4 => definition = usize::from(reader.u16()?),
            5 => definition = reader.u32()? as usize,
            6 => {
                reader.take(1)?;
            }
            7 | 8 => {
                let start = reader.u16()?;
                let end = if command == 8 {
                    reader.u16()?
                } else {
                    start.checked_add(1).ok_or("DSF object index overflow")?
                };
                if end < start {
                    return Err("Invalid DSF object range".into());
                }
                let pool = pools
                    .get(current.ok_or("DSF object has no pool")?)
                    .filter(|pool| matches!(pool.depth, 3 | 4))
                    .ok_or("DSF object has an invalid coordinate pool")?;
                let resource = object_names
                    .get(definition)
                    .ok_or("DSF object definition is missing")?;
                for index in start..end {
                    if objects.len() >= 100_000 {
                        return Err("DSF object placement budget exceeded".into());
                    }
                    let point = pool
                        .points
                        .get(usize::from(index))
                        .ok_or("DSF object point is missing")?;
                    if point[..pool.depth].iter().any(|v| !v.is_finite())
                        || point[0] < bounds[0] - 0.001
                        || point[0] > bounds[1] + 0.001
                        || point[1] < bounds[2] - 0.001
                        || point[1] > bounds[3] + 0.001
                    {
                        return Err("DSF object coordinate is invalid".into());
                    }
                    objects.push(Placement {
                        resource: resource.clone(),
                        coordinate: [point[1], point[0]],
                        heading: point[2],
                        height: (pool.depth == 4).then_some(point[3]),
                    });
                }
            }
            9..=11 => {
                // A network chain is a road, rail or water centreline. Its third plane is a
                // height above the draped terrain rather than an absolute elevation, despite the
                // specification describing DSF vertical values as metres above mean sea level;
                // the installed global scenery publishes 0 to 2 m over terrain above 4,000 m, so
                // the absolute reading is impossible. The chain is retained here and resolved
                // against the mesh once the tile is indexed; command 11 carries explicit 32-bit
                // indices and takes no junction offset.
                let mut indices: Vec<u32> = Vec::new();
                if command == 10 {
                    let start = u32::from(reader.u16()?);
                    let end = u32::from(reader.u16()?);
                    if end < start {
                        return Err("Invalid DSF network range".into());
                    }
                    indices.extend(start..end);
                } else {
                    let count = usize::from(reader.u8()?);
                    for _ in 0..count {
                        indices.push(if command == 9 {
                            u32::from(reader.u16()?)
                        } else {
                            reader.u32()?
                        });
                    }
                }
                if command != 11 {
                    for index in &mut indices {
                        *index = index
                            .checked_add(junction_offset)
                            .ok_or("DSF network junction index overflow")?;
                    }
                }
                let pool = network_pools
                    .get(current.ok_or("DSF network has no coordinate pool")?)
                    .filter(|pool| pool.depth >= 3)
                    .ok_or("DSF network has an invalid coordinate pool")?;
                let mut chain = Vec::with_capacity(indices.len());
                for index in indices {
                    let point = *pool
                        .points
                        .get(index as usize)
                        .ok_or("DSF network point is missing")?;
                    if point[..3].iter().any(|value| !value.is_finite()) {
                        return Err("DSF network coordinate is invalid".into());
                    }
                    // Store as latitude, longitude, elevation, matching the rest of the planner.
                    chain.push([point[1], point[0], point[2]]);
                }
                network_points += chain.len();
                if network_points > 4 * 1024 * 1024 {
                    return Err("DSF network points exceed their budget".into());
                }
                networks.push(chain);
            }
            12..=15 => {
                let (parameter, indices) = polygon_indices(&mut reader, command)?;
                if facade(&polygon_names, definition) || forest(&polygon_names, definition) {
                    // A facade is a building: its parameter is the height in metres and its
                    // rings are the footprint. Retaining it here is what lets the scenery
                    // loader treat it as an obstacle instead of an unknown obstruction.
                    if facades.len() >= 100_000 {
                        return Err("DSF facade count exceeds its budget".into());
                    }
                    let pool = pools
                        .get(current.ok_or("DSF facade has no pool")?)
                        .filter(|pool| pool.depth >= 2)
                        .ok_or("DSF facade has an invalid coordinate pool")?;
                    let mut rings = Vec::new();
                    for indices in indices {
                        polygon_points += indices.len();
                        if polygon_points > MAX_POLYGON_POINTS {
                            return Err("DSF facade points exceed their budget".into());
                        }
                        let mut ring = Vec::new();
                        for index in indices {
                            let point = *pool
                                .points
                                .get(usize::from(index))
                                .ok_or("DSF facade point is missing")?;
                            // A facade may draw its footprint from a shared point pool that
                            // spans more than this tile, so the tile bounds shown by the draped
                            // path are not a valid test here. Only reject coordinates that
                            // cannot be a location. Plane 0 is longitude and plane 1 latitude,
                            // matching the draped path.
                            if point.iter().any(|value| !value.is_finite())
                                || point[0].abs() > 180.0
                                || point[1].abs() >= 85.0
                            {
                                return Err("DSF facade coordinate is invalid".into());
                            }
                            // Store as latitude then longitude, matching the rest of the planner.
                            ring.push([point[1], point[0]]);
                        }
                        rings.push(ring);
                    }
                    let forest = forest(&polygon_names, definition);
                    facades.push(Facade {
                        resource: std::sync::Arc::<str>::from(polygon_names[definition]),
                        rings,
                        height_meters: if forest { 0.0 } else { f64::from(parameter) },
                        forest,
                    });
                    continue;
                }
                if let Some(autogen) = string_definition(&polygon_names, definition) {
                    // A string is a line or area of repeated objects. Its extent is the polygon
                    // footprint and its height comes from the catalogue, so the rings are
                    // retained now and resolved once the tile is indexed.
                    if strings.len() >= 100_000 {
                        return Err("DSF string count exceeds its budget".into());
                    }
                    let pool = pools
                        .get(current.ok_or("DSF string has no pool")?)
                        .filter(|pool| pool.depth >= 2)
                        .ok_or("DSF string has an invalid coordinate pool")?;
                    let mut rings = Vec::new();
                    for indices in indices {
                        polygon_points += indices.len();
                        if polygon_points > MAX_POLYGON_POINTS {
                            return Err("DSF string points exceed their budget".into());
                        }
                        let mut ring = Vec::new();
                        for index in indices {
                            let point = *pool
                                .points
                                .get(usize::from(index))
                                .ok_or("DSF string point is missing")?;
                            if point.iter().any(|value| !value.is_finite())
                                || point[0].abs() > 180.0
                                || point[1].abs() >= 85.0
                            {
                                return Err("DSF string coordinate is invalid".into());
                            }
                            // Store as latitude then longitude, matching the rest of the planner.
                            ring.push([point[1], point[0]]);
                        }
                        rings.push(ring);
                    }
                    strings.push(StringPolygon {
                        resource: std::sync::Arc::<str>::from(polygon_names[definition]),
                        rings,
                        autogen,
                        height_meters: 0.0,
                        reach_meters: 0.0,
                        ground_reach_meters: None,
                    });
                    continue;
                }
                unresolved_obstacles |= !draped(&polygon_names, definition);
                if draped(&polygon_names, definition) {
                    if polygons.len() >= 100_000 {
                        return Err("DSF draped polygon count exceeds its budget".into());
                    }
                    let pool = pools
                        .get(current.ok_or("DSF polygon has no pool")?)
                        .filter(|pool| pool.depth >= 2)
                        .ok_or("DSF polygon has an invalid coordinate pool")?;
                    let mut rings = Vec::new();
                    for indices in indices {
                        polygon_points += indices.len();
                        if polygon_points > MAX_POLYGON_POINTS {
                            return Err("DSF draped polygon points exceed their budget".into());
                        }
                        let mut ring = Vec::new();
                        for index in indices {
                            let point = *pool
                                .points
                                .get(usize::from(index))
                                .ok_or("DSF polygon point is missing")?;
                            if point.iter().any(|value| !value.is_finite())
                                || point[0] < bounds[0] - 0.001
                                || point[0] > bounds[1] + 0.001
                                || point[1] < bounds[2] - 0.001
                                || point[1] > bounds[3] + 0.001
                            {
                                return Err("DSF polygon coordinate is invalid".into());
                            }
                            ring.push(point);
                        }
                        rings.push(ring);
                    }
                    polygons.push(DrapedPolygon {
                        airport: airport_filter.clone(),
                        resource: polygon_resources
                            .entry(definition)
                            .or_insert_with(|| {
                                std::sync::Arc::<str>::from(polygon_names[definition])
                            })
                            .clone(),
                        parameter,
                        depth: pool.depth,
                        rings,
                    });
                }
            }
            16 => {}
            17 | 18 => {
                let flags = reader.u8()?;
                if flags & !3 != 0 {
                    return Err("Unsupported DSF terrain flags".into());
                }
                physical = flags & 1 != 0;
                if command == 18 {
                    reader.take(8)?;
                }
            }
            23..=31 => {
                let kind = (command - 23) / 3;
                let form = (command - 23) % 3;
                let mut indices = Vec::new();
                if form == 2 {
                    let start = reader.u16()?;
                    let end = reader.u16()?;
                    if end < start {
                        return Err("Invalid DSF triangle range".into());
                    }
                    let selected = current.ok_or("DSF mesh has no coordinate pool")?;
                    indices.extend((start..end).map(|index| (selected, usize::from(index))));
                } else {
                    let n = reader.u8()?;
                    for _ in 0..n {
                        let selected = if form == 1 {
                            usize::from(reader.u16()?)
                        } else {
                            current.ok_or("DSF mesh has no coordinate pool")?
                        };
                        indices.push((selected, usize::from(reader.u16()?)));
                    }
                }
                if kind == 0 && indices.len() % 3 != 0 || indices.len() < 3 {
                    return Err("Invalid DSF triangle primitive".into());
                }
                let mut add = |corners: [(usize, usize); 3]| -> Result<(), String> {
                    if !physical {
                        return Ok(());
                    }
                    if triangles.len() >= 2 * 1024 * 1024 {
                        return Err("DSF physical triangle budget exceeded".into());
                    }
                    let mut triangle = [Vertex {
                        longitude: 0.0,
                        latitude: 0.0,
                        meters: 0.0,
                    }; 3];
                    for (slot, (pool, index)) in triangle.iter_mut().zip(corners) {
                        let pool = pools
                            .get(pool)
                            .filter(|pool| pool.depth >= 5)
                            .ok_or("DSF terrain references a non-mesh pool")?;
                        let p = *pool
                            .points
                            .get(index)
                            .ok_or("DSF triangle vertex is out of range")?;
                        if p[0] < bounds[0] - 0.001
                            || p[0] > bounds[1] + 0.001
                            || p[1] < bounds[2] - 0.001
                            || p[1] > bounds[3] + 0.001
                        {
                            return Err("DSF triangle lies outside its tile".into());
                        }
                        let dem = raster
                            .as_ref()
                            .map(|r| r.upper([p[0], p[1]], bounds))
                            .transpose()?;
                        let height = if (p[2] + 32768.0).abs() < 0.1 {
                            dem.ok_or("DSF mesh references unavailable elevation raster")?
                        } else {
                            dem.map_or(p[2], |dem| p[2].max(dem))
                        };
                        if !(-1500.0..=12000.0).contains(&height) {
                            return Err("DSF mesh elevation is invalid".into());
                        }
                        *slot = Vertex {
                            longitude: p[0],
                            latitude: p[1],
                            meters: height,
                        };
                    }
                    triangles.push(triangle);
                    Ok(())
                };
                match kind {
                    0 => {
                        for tri in indices.as_chunks::<3>().0 {
                            add([tri[0], tri[1], tri[2]])?;
                        }
                    }
                    1 => {
                        for tri in indices.windows(3) {
                            add([tri[0], tri[1], tri[2]])?;
                        }
                    }
                    2 => {
                        for pair in indices[1..].windows(2) {
                            add([indices[0], pair[0], pair[1]])?;
                        }
                    }
                    _ => unreachable!(),
                }
            }
            32..=34 => {
                let n = match command {
                    32 => u32::from(reader.u8()?),
                    33 => u32::from(reader.u16()?),
                    _ => reader.u32()?,
                };
                let comment = reader.take(n as usize)?;
                if let Some(filter) = filter_comment(comment, &airport_filters)? {
                    airport_filter = filter;
                }
            }
            _ => {
                return Err(format!(
                    "Unsupported DSF placement command {command} at command byte {}",
                    reader.at - 1
                ));
            }
        }
        if matches!(command, 12..=15) && draped(&polygon_names, definition) {
            draped_resources.insert(std::sync::Arc::<str>::from(polygon_names[definition]));
        }
    }
    let mut tile = Tile {
        bounds,
        overlay: property("sim/overlay") == Some("1"),
        // Object placements are not resolved here because their resources live outside the tile.
        // The loader resolves every retained placement and sets this flag when one cannot be
        // loaded, so a tile whose placements all resolve is understood and a tile with any
        // unloadable placement still refuses release.
        unresolved_obstacles,
        unresolved_classes: std::collections::BTreeSet::new(),
        unresolved_objects: Vec::new(),
        unresolved_objects_overflow: false,
        objects,
        obstacles: Vec::new(),
        obstacle_bins: Vec::new(),
        draped_resources,
        polygons,
        facades,
        strings,
        networks,
        polygon_exclusions,
        triangles,
        bins: Vec::new(),
    };
    tile.index()?;
    Ok(tile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facades_and_ground_polygons_are_told_apart_by_their_definition() {
        // X-Plane decides whether a polygon is a building or ground from the definition file
        // extension, so the loader must use the same rule.
        let names = [
            "buildings.fac",
            "apron.pol",
            "road.lin",
            "trees.for",
            "coast.bch",
        ];
        assert!(facade(&names, 0));
        assert!(!facade(&names, 1));
        assert!(!facade(&names, 2));
        assert!(!facade(&names, 3));
        // Case is not significant in a scenery package reference.
        assert!(facade(&["BUILDINGS.FAC"], 0));
        assert!(draped(&names, 1));
        assert!(draped(&names, 2));
        assert!(!draped(&names, 0));
        assert!(!draped(&names, 3));
        // A beach is a flat shoreline surface, not a vertical obstacle.
        assert!(draped(&names, 4));
        assert!(!facade(&names, 4));
        assert_eq!(string_definition(&["lights.str"], 0), Some(false));
        assert_eq!(string_definition(&["block.agb"], 0), Some(true));
        assert_eq!(string_definition(&["block.ags"], 0), Some(true));
        // An unknown extension is neither, and must stay unresolved rather than assumed.
        assert!(!facade(&["thing.xyz"], 0));
        assert!(!draped(&["thing.xyz"], 0));
    }

    #[test]
    fn airport_filter_comments_select_clear_and_reject_missing_ids() {
        let filters = [
            std::sync::Arc::<str>::from("KMEM"),
            std::sync::Arc::from("KSDF"),
        ];
        for (index, expected) in [(0i32, Some("KMEM")), (1, Some("KSDF")), (-1, None)] {
            let bytes = [
                1u16.to_le_bytes().as_slice(),
                index.to_le_bytes().as_slice(),
            ]
            .concat();
            let result = filter_comment(&bytes, &filters).unwrap().unwrap();
            assert_eq!(result.as_deref(), expected);
        }
        assert_eq!(filter_comment(&[2, 0], &filters).unwrap(), None);
        assert!(filter_comment(&[1, 0, 0], &filters).is_err());
        for index in [-2i32, 2, i32::MAX] {
            let bytes = [
                1u16.to_le_bytes().as_slice(),
                index.to_le_bytes().as_slice(),
            ]
            .concat();
            assert!(filter_comment(&bytes, &filters).is_err());
        }
    }

    #[test]
    fn polygon_commands_preserve_holes_and_half_open_ranges() {
        for (command, bytes) in [
            (12, vec![42, 0, 3, 2, 0, 4, 0, 6, 0]),
            (13, vec![42, 0, 2, 0, 5, 0]),
            (14, vec![42, 0, 2, 3, 0, 0, 1, 0, 2, 0, 3, 3, 0, 4, 0, 5, 0]),
            (15, vec![42, 0, 2, 0, 0, 3, 0, 6, 0]),
        ] {
            let mut reader = Reader::new(&bytes);
            let (parameter, rings) = polygon_indices(&mut reader, command).unwrap();
            assert_eq!(parameter, 42);
            assert_eq!(reader.at, bytes.len());
            if command >= 14 {
                assert_eq!(rings, vec![vec![0, 1, 2], vec![3, 4, 5]]);
            } else if command == 13 {
                assert_eq!(rings, vec![vec![2, 3, 4]]);
            } else {
                assert_eq!(rings, vec![vec![2, 4, 6]]);
            }
            assert!(polygon_indices(&mut Reader::new(&bytes[..bytes.len() - 1]), command).is_err());
        }
        assert!(polygon_indices(&mut Reader::new(&[0, 0, 5, 0, 2, 0]), 13).is_err());
        assert!(polygon_indices(&mut Reader::new(&[0, 0, 1, 5, 0, 2, 0]), 15).is_err());
    }

    #[test]
    fn coordinate_delta_and_run_decoding_wraps_without_overrunning_the_pool() {
        let expected = [65530u16, 5, 10];
        let scale = [1.0f32.to_le_bytes(), 0.0f32.to_le_bytes()].concat();
        for encoding in 0..=3u8 {
            let mut bytes = 3u32.to_le_bytes().to_vec();
            bytes.extend([1, encoding]);
            if encoding & 2 != 0 {
                bytes.push(3);
            }
            let values = if encoding & 1 == 0 {
                expected
            } else {
                [65530, 11, 5]
            };
            for value in values {
                bytes.extend(value.to_le_bytes());
            }
            let decoded = pool(&bytes, &scale, 2).unwrap();
            for (index, expected) in expected.iter().enumerate() {
                assert!((decoded.points[index][0] - f64::from(*expected) / 65535.0).abs() < 1e-12);
            }
            assert!(pool(&bytes[..bytes.len() - 1], &scale, 2).is_err());
        }
        for run in [0, 4, 128, 132] {
            let mut bytes = 3u32.to_le_bytes().to_vec();
            bytes.extend([1, 2, run, 0, 0]);
            assert!(pool(&bytes, &scale, 2).is_err());
        }
        let mut repeat = 3u32.to_le_bytes().to_vec();
        repeat.extend([1, 2, 131, 7, 0]);
        assert!(
            pool(&repeat, &scale, 2)
                .unwrap()
                .points
                .iter()
                .all(|point| (point[0] - 7.0 / 65535.0).abs() < 1e-12)
        );
    }

    #[test]
    fn vector_pools_decode_32_bit_points_and_tolerate_an_empty_pool() {
        // A 32-bit pool normalises over 2^32 - 1 so the same scale convention as the 16-bit
        // pools still maps onto the published range.
        let mut bytes = 2u32.to_le_bytes().to_vec();
        bytes.extend([1, 0]); // one plane, raw encoding
        bytes.extend(u32::MAX.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        let scale = [1.0f32.to_le_bytes(), 7.0f32.to_le_bytes()].concat();
        let decoded = pool(&bytes, &scale, 4).unwrap();
        assert!((decoded.points[0][0] - 8.0).abs() < 1e-12);
        assert!((decoded.points[1][0] - 7.0).abs() < 1e-12);
        // Differencing accumulates over the full 32-bit range, unlike the 16-bit pools.
        let mut wrapped = 2u32.to_le_bytes().to_vec();
        wrapped.extend([1, 1]);
        wrapped.extend(70000u32.to_le_bytes());
        wrapped.extend(1u32.to_le_bytes());
        let decoded = pool(&wrapped, &scale, 4).unwrap();
        assert!((decoded.points[0][0] - (7.0 + 70000.0 / 4_294_967_295.0)).abs() < 1e-12);
        assert!((decoded.points[1][0] - (7.0 + 70001.0 / 4_294_967_295.0)).abs() < 1e-12);
        // The default scenery appends an empty 32-bit pool; it must not fail the whole tile.
        let empty = [0u8; 4].into_iter().chain([0u8]).collect::<Vec<_>>();
        assert!(pool(&empty, &[], 4).unwrap().points.is_empty());
        // A pool with points must still declare a usable depth and matching scale.
        assert!(pool(&[1, 0, 0, 0, 0], &[], 4).is_err());
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn raster_offsets_and_pixel_formats_keep_unknown_heights_out_of_the_mesh() {
        let mut info = vec![1, 2];
        info.extend(5u16.to_le_bytes());
        info.extend(2u32.to_le_bytes());
        info.extend(2u32.to_le_bytes());
        info.extend(2.0f32.to_le_bytes());
        info.extend(10.0f32.to_le_bytes());
        let values = [0i16, 10, 20, 30]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        let raster = Raster::read(&info, &values).unwrap();
        assert!(raster.post);
        assert_eq!(
            raster.upper([0.5, 0.5], [0.0, 1.0, 0.0, 1.0]).unwrap(),
            70.0
        );
        assert!(Raster::read(&info, &values[..values.len() - 1]).is_err());
        let mut bad = info.clone();
        bad[2..4].copy_from_slice(&7u16.to_le_bytes());
        assert!(Raster::read(&bad, &values).is_err());
        let unknown = [i16::MIN; 4]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>();
        assert!(Raster::read(&info, &unknown).is_err());
    }
}
