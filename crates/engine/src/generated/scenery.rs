//! Installed physical scenery mesh, read on planning workers without SDK calls.
//! Mesh clearance alone does not establish obstacle or controlled-airspace protection.
mod dsf;
mod library;
mod object;
use openatc_core::traffic::motion::Waypoint;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_BYTES: usize = 128 * 1024 * 1024;
const GRID: usize = 128;

/// The enabled stack is retained for one planning job; no runtime scenery-folder scans.
pub(super) struct Terrain {
    packs: Vec<PathBuf>,
    tiles: BTreeMap<(i32, i32), Result<Vec<Arc<Tile>>, String>>,
    retained_bytes: usize,
    obstacle_seen: bool,
    library: library::Library,
}
impl Terrain {
    pub fn new(root: &Path) -> Result<Self, String> {
        let path = root.join("Custom Scenery/scenery_packs.ini");
        let mut packs = Vec::new();
        if path.is_file() {
            if path.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 {
                return Err("Enabled scenery list exceeds the planning budget".into());
            }
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            for line in text.lines() {
                let Some(name) = line.strip_prefix("SCENERY_PACK ").map(str::trim) else {
                    continue;
                };
                if packs.len() >= 1024 || name.len() > 8192 {
                    return Err("Enabled scenery count exceeds the planning budget".into());
                }
                let path = if name == "*GLOBAL_AIRPORTS*" {
                    root.join("Global Scenery/Global Airports")
                } else if Path::new(name).is_absolute() {
                    PathBuf::from(name)
                } else {
                    root.join(name)
                };
                if !packs.contains(&path) {
                    packs.push(path);
                }
            }
        }
        let base = root.join("Global Scenery/X-Plane 12 Global Scenery");
        if !packs.contains(&base) {
            packs.push(base);
        }
        let library = library::Library::load(root, &packs)?;
        Ok(Self {
            packs,
            tiles: BTreeMap::new(),
            retained_bytes: 0,
            obstacle_seen: false,
            library,
        })
    }

    pub fn supplement_airport(
        &self,
        airport: &mut openatc_core::airport::Airport,
    ) -> Result<usize, String> {
        let tiles = airport_tiles(airport)?;
        let mut added = 0;
        for (latitude, longitude) in tiles {
            added += self.supplement_airport_tile(airport, latitude, longitude)?;
        }
        Ok(added)
    }

    fn supplement_airport_tile(
        &self,
        airport: &mut openatc_core::airport::Airport,
        latitude: i32,
        longitude: i32,
    ) -> Result<usize, String> {
        let owner = Path::new(&airport.source)
            .parent()
            .and_then(Path::parent)
            .ok_or("Airport has no owning scenery package")?;
        let directory = format!(
            "{:+03}{:+04}",
            latitude.div_euclid(10) * 10,
            longitude.div_euclid(10) * 10
        );
        let filename = format!("{latitude:+03}{longitude:+04}.dsf");
        let mut exclusions: Vec<[f64; 4]> = Vec::new();
        let mut surfaces = BTreeMap::new();
        let mut added = 0;
        for pack in &self.packs {
            let path = pack.join("Earth nav data").join(&directory).join(&filename);
            if !path.is_file() {
                continue;
            }
            let tile = dsf::parse(&file_bytes(&path)?)?;
            if pack == owner {
                for polygon in &tile.polygons {
                    if polygon.airport.as_deref() != Some(airport.icao.as_str()) {
                        continue;
                    }
                    let curved = match (polygon.parameter == 65535, polygon.depth) {
                        (false, 2) | (true, 4) => false,
                        (false, 4) | (true, 8) => true,
                        _ => continue,
                    };
                    let bounds = polygon.rings.iter().flatten().fold(
                        [
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                            f64::INFINITY,
                            f64::NEG_INFINITY,
                        ],
                        |mut bounds, point| {
                            bounds[0] = bounds[0].min(point[0]);
                            bounds[1] = bounds[1].max(point[0]);
                            bounds[2] = bounds[2].min(point[1]);
                            bounds[3] = bounds[3].max(point[1]);
                            if curved {
                                for control in [
                                    [point[2], point[3]],
                                    [2.0 * point[0] - point[2], 2.0 * point[1] - point[3]],
                                ] {
                                    bounds[0] = bounds[0].min(control[0]);
                                    bounds[1] = bounds[1].max(control[0]);
                                    bounds[2] = bounds[2].min(control[1]);
                                    bounds[3] = bounds[3].max(control[1]);
                                }
                            }
                            bounds
                        },
                    );
                    // Using the exclusion's bounding rectangle may reject extra pavement, never restore excluded pavement.
                    if exclusions.iter().any(|excluded| {
                        bounds[0] <= excluded[1]
                            && bounds[1] >= excluded[0]
                            && bounds[2] <= excluded[3]
                            && bounds[3] >= excluded[2]
                    }) {
                        continue;
                    }
                    let paved = *surfaces.entry(polygon.resource.clone()).or_insert_with(|| {
                        self.library
                            .paved_surface(pack, &polygon.resource)
                            .unwrap_or(false)
                    });
                    if !paved {
                        continue;
                    }
                    let rings = polygon
                        .rings
                        .iter()
                        .map(|ring| {
                            ring.iter()
                                .map(|point| {
                                    let node = openatc_core::airport::airport_point(
                                        airport, point[1], point[0],
                                    );
                                    let control = curved.then(|| {
                                        openatc_core::airport::airport_point(
                                            airport, point[3], point[2],
                                        )
                                    });
                                    (node, control)
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>();
                    if openatc_core::airport::append_pavement(airport, &rings).is_ok() {
                        added += 1;
                    }
                }
                break;
            }
            exclusions.extend(tile.polygon_exclusions);
        }
        Ok(added)
    }

    fn tile(&mut self, latitude: i32, longitude: i32) -> Result<&[Arc<Tile>], String> {
        let key = (latitude, longitude);
        if !self.tiles.contains_key(&key) {
            if self.tiles.len() >= 24 {
                return Err("Terminal scenery tile budget exceeded".into());
            }
            let directory = format!(
                "{:+03}{:+04}",
                latitude.div_euclid(10) * 10,
                longitude.div_euclid(10) * 10
            );
            let filename = format!("{latitude:+03}{longitude:+04}.dsf");
            let mut added_bytes = 0;
            let result = (|| {
                let mut tiles = Vec::new();
                let mut base = false;
                for pack in &self.packs {
                    let path = pack.join("Earth nav data").join(&directory).join(&filename);
                    if !path.is_file() {
                        continue;
                    }
                    let tile = Tile::load_with(&path, Some(&self.library))
                        .map_err(|error| format!("{}: {error}", path.display()))?;
                    if tile.bounds
                        != [
                            f64::from(longitude),
                            f64::from(longitude + 1),
                            f64::from(latitude),
                            f64::from(latitude + 1),
                        ]
                    {
                        return Err("Scenery file bounds do not match its requested tile".into());
                    }
                    added_bytes += tile.triangles.len() * std::mem::size_of::<[Vertex; 3]>()
                        + tile
                            .bins
                            .iter()
                            .map(|bin| bin.capacity() * std::mem::size_of::<u32>())
                            .sum::<usize>()
                        + tile.bins.capacity() * std::mem::size_of::<Vec<u32>>();
                    added_bytes += tile.object_bytes();
                    if self.retained_bytes + added_bytes > 256 * 1024 * 1024 {
                        return Err("Terminal mesh cache exceeds 256 MiB".into());
                    }
                    base = !tile.overlay;
                    tiles.push(Arc::new(tile));
                    if base {
                        break;
                    }
                }
                if !base {
                    return Err(format!(
                        "No enabled physical base mesh for {latitude:+03}{longitude:+04}"
                    ));
                }
                Ok(tiles)
            })();
            if result.is_ok() {
                self.retained_bytes += added_bytes;
            }
            self.tiles.insert(key, result);
        }
        self.tiles[&key]
            .as_ref()
            .map(Vec::as_slice)
            .map_err(Clone::clone)
    }

    /// Terrain-only result; callers must separately resolve obstacles and controller authority.
    pub fn terrain_clear(
        &mut self,
        from: Waypoint,
        to: Waypoint,
        margin: f64,
    ) -> Result<bool, String> {
        if [from, to].iter().any(|p| {
            !p.latitude.is_finite()
                || !p.longitude.is_finite()
                || p.latitude.abs() >= 85.0
                || p.longitude.abs() >= 180.0
        }) || (from.latitude - to.latitude).abs() > 1.0
            || (from.longitude - to.longitude).abs() > 1.0
        {
            return Err("Terrain corridor exceeds its coordinate or edge budget".into());
        }
        let mut cuts = vec![0.0, 1.0];
        for (a, b) in [(from.latitude, to.latitude), (from.longitude, to.longitude)] {
            if (a - b).abs() < 1e-12 {
                continue;
            }
            for edge in a.min(b).ceil() as i32..=a.max(b).floor() as i32 {
                let fraction = (f64::from(edge) - a) / (b - a);
                if fraction > 0.0 && fraction < 1.0 {
                    cuts.push(fraction);
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let interpolate = |t: f64| Waypoint {
            latitude: from.latitude + (to.latitude - from.latitude) * t,
            longitude: from.longitude + (to.longitude - from.longitude) * t,
            altitude_feet: from.altitude_feet + (to.altitude_feet - from.altitude_feet) * t,
        };
        for pair in cuts.windows(2) {
            let middle = interpolate((pair[0] + pair[1]) * 0.5);
            let stack = self
                .tile(
                    middle.latitude.floor() as i32,
                    middle.longitude.floor() as i32,
                )?
                .to_vec();
            self.obstacle_seen |= stack.iter().any(|tile| tile.unresolved_obstacles);
            let a = interpolate(pair[0]);
            let b = interpolate(pair[1]);
            let mut covered = false;
            for tile in &stack {
                match tile.terrain_clear(a, b, margin) {
                    Some(false) => return Ok(false),
                    Some(true) if !tile.overlay => covered = true,
                    _ => {}
                }
            }
            if !covered {
                return Err("Physical scenery mesh leaves a gap in the terrain corridor".into());
            }
        }
        Ok(true)
    }

    pub fn has_unresolved_obstacles(&self) -> bool {
        self.obstacle_seen
    }

    pub fn obstacles_clear(
        &mut self,
        from: Waypoint,
        to: Waypoint,
        radius: f64,
    ) -> Result<bool, String> {
        if !(0.0..=100.0).contains(&radius)
            || [from, to].iter().any(|point| {
                !point.latitude.is_finite()
                    || !point.longitude.is_finite()
                    || !point.altitude_feet.is_finite()
                    || point.latitude.abs() >= 85.0
                    || point.longitude.abs() >= 180.0
            })
            || (from.latitude - to.latitude).abs() > 1.0
            || (from.longitude - to.longitude).abs() > 1.0
        {
            return Err("Invalid aircraft obstacle envelope".into());
        }
        let mut stacks = Vec::new();
        for latitude in from.latitude.min(to.latitude).floor() as i32
            ..=from.latitude.max(to.latitude).floor() as i32
        {
            for longitude in from.longitude.min(to.longitude).floor() as i32
                ..=from.longitude.max(to.longitude).floor() as i32
            {
                stacks.push(self.tile(latitude, longitude)?.to_vec());
            }
        }
        for stack in stacks {
            for tile in &stack {
                self.obstacle_seen |= tile.unresolved_obstacles;
                for obstacle in &tile.obstacles {
                    let placement = &obstacle.placement;
                    let ground = stack
                        .iter()
                        .filter_map(|layer| layer.mesh_height(placement.coordinate))
                        .reduce(f64::max);
                    let Some(ground) = ground else {
                        self.obstacle_seen = true;
                        continue;
                    };
                    // Four-plane placements may be absolute or use the DSF AGL placement comment.
                    let anchor = placement
                        .height
                        .map_or(ground, |height| height.max(ground + height));
                    if !obstacle.clear(from, to, anchor, radius) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
    pub fn begin_check(&mut self) {
        self.obstacle_seen = false;
    }
}

fn airport_tiles(airport: &openatc_core::airport::Airport) -> Result<Vec<(i32, i32)>, String> {
    let mut bounds = [
        airport.reference_latitude,
        airport.reference_latitude,
        airport.reference_longitude,
        airport.reference_longitude,
    ];
    for point in airport
        .nodes
        .iter()
        .map(|node| node.point)
        .chain(airport.parking.iter().map(|stand| stand.point))
        .chain(
            airport
                .runways
                .iter()
                .flat_map(|runway| [runway.first, runway.second]),
        )
    {
        let coordinate = super::position(airport, point);
        if coordinate.iter().any(|value| !value.is_finite()) {
            return Err("Airport surface tile extent is invalid".into());
        }
        bounds[0] = bounds[0].min(coordinate[0]);
        bounds[1] = bounds[1].max(coordinate[0]);
        bounds[2] = bounds[2].min(coordinate[1]);
        bounds[3] = bounds[3].max(coordinate[1]);
    }
    // Include room for the aircraft and checked connector curves at the outermost graph nodes.
    let latitude_margin = 500.0 / 111320.0;
    let longitude_margin = latitude_margin / airport.reference_latitude.to_radians().cos();
    bounds[0] -= latitude_margin;
    bounds[1] += latitude_margin;
    bounds[2] -= longitude_margin;
    bounds[3] += longitude_margin;
    if bounds.iter().any(|value| !value.is_finite())
        || bounds[0] < -85.0
        || bounds[1] >= 85.0
        || bounds[2] < -180.0
        || bounds[3] >= 180.0
        || bounds[1] - bounds[0] > 1.0
        || bounds[3] - bounds[2] > 1.0
    {
        return Err("Airport surface exceeds its supported tile extent".into());
    }
    let mut tiles = Vec::new();
    for latitude in bounds[0].floor() as i32..=bounds[1].floor() as i32 {
        for longitude in bounds[2].floor() as i32..=bounds[3].floor() as i32 {
            tiles.push((latitude, longitude));
        }
    }
    if tiles.len() > 4 {
        return Err("Airport surface tile count exceeds its budget".into());
    }
    Ok(tiles)
}

fn file_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = path.metadata().map_err(|e| e.to_string())?;
    if metadata.len() > MAX_BYTES as u64 {
        return Err("Scenery tile exceeds the input budget".into());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut prefix = [0; 8];
    (&file).read_exact(&mut prefix).map_err(|e| e.to_string())?;
    if &prefix == b"XPLNEDSF" {
        let mut output = prefix.to_vec();
        file.take(MAX_BYTES as u64 + 1 - prefix.len() as u64)
            .read_to_end(&mut output)
            .map_err(|e| e.to_string())?;
        if output.len() > MAX_BYTES {
            return Err("Scenery tile exceeds the input budget".into());
        }
        return Ok(output);
    }
    if prefix[..6] != [0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c] {
        return Err("Scenery tile is neither DSF nor a supported 7z archive".into());
    }
    let mut archive =
        sevenz_rust2::ArchiveReader::open(path, Default::default()).map_err(|e| e.to_string())?;
    archive.set_thread_count(1);
    let entries = &archive.archive().files;
    if entries.len() != 1
        || entries[0].is_directory
        || entries[0].size > MAX_BYTES as u64
        || !entries[0].name.ends_with(".dsf")
        || archive.archive().blocks.iter().any(|block| {
            block.get_unpack_size() > MAX_BYTES as u64
                || block
                    .coders
                    .iter()
                    .any(|coder| match coder.encoder_method_id() {
                        [0] => false,
                        [3, 1, 1] => {
                            coder.properties().len() != 5
                                || u32::from_le_bytes(
                                    coder
                                        .properties()
                                        .get(1..5)
                                        .unwrap_or(&[255; 4])
                                        .try_into()
                                        .unwrap(),
                                ) > 64 * 1024 * 1024
                        }
                        [0x21] => coder.properties().len() != 1 || coder.properties()[0] > 26,
                        _ => true,
                    })
        })
    {
        return Err("Scenery archive exceeds its file, codec or dictionary budget".into());
    }
    let mut output = Vec::new();
    archive
        .for_each_entries(|_, reader| {
            reader
                .take(MAX_BYTES as u64 + 1)
                .read_to_end(&mut output)
                .map_err(|error| sevenz_rust2::Error::Io(error, "scenery tile".into()))?;
            Ok(true)
        })
        .map_err(|e| e.to_string())?;
    if output.len() > MAX_BYTES {
        return Err("Expanded scenery tile exceeds the input budget".into());
    }
    Ok(output)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Vertex {
    pub longitude: f64,
    pub latitude: f64,
    pub meters: f64,
}

#[derive(Clone)]
struct Placement {
    resource: Arc<str>,
    coordinate: [f64; 2],
    heading: f64,
    height: Option<f64>,
}
struct Obstacle {
    placement: Placement,
    geometry: Arc<Vec<object::Bounds>>,
}
impl Obstacle {
    fn clear(&self, from: Waypoint, to: Waypoint, anchor: f64, radius: f64) -> bool {
        let (sin, cos) = self.placement.heading.to_radians().sin_cos();
        let local = |point: Waypoint| {
            let east = (point.longitude - self.placement.coordinate[1])
                * 111320.0
                * self.placement.coordinate[0].to_radians().cos();
            let north = (point.latitude - self.placement.coordinate[0]) * 111320.0;
            [
                east * cos - north * sin,
                point.altitude_feet * 0.3048,
                -east * sin - north * cos,
            ]
        };
        let a = local(from);
        let b = local(to);
        let padding =
            radius + 1.0 + 0.02 * a[0].abs().max(a[2].abs()).max(b[0].abs()).max(b[2].abs());
        self.geometry.iter().all(|bounds| {
            let mut low: f64 = 0.0;
            let mut high: f64 = 1.0;
            for axis in [0, 2] {
                let delta = b[axis] - a[axis];
                if delta.abs() < 1e-12 {
                    if a[axis] < bounds.low[axis] - padding || a[axis] > bounds.high[axis] + padding
                    {
                        return true;
                    }
                } else {
                    let first = (bounds.low[axis] - padding - a[axis]) / delta;
                    let last = (bounds.high[axis] + padding - a[axis]) / delta;
                    low = low.max(first.min(last));
                    high = high.min(first.max(last));
                }
            }
            low > high
                || (a[1] + (b[1] - a[1]) * low).min(a[1] + (b[1] - a[1]) * high)
                    > anchor + bounds.high[1]
        })
    }
}

struct DrapedPolygon {
    airport: Option<Arc<str>>,
    resource: Arc<str>,
    parameter: u16,
    depth: usize,
    rings: Vec<Vec<[f64; 4]>>,
}

pub(super) struct Tile {
    pub bounds: [f64; 4],
    pub overlay: bool,
    pub unresolved_obstacles: bool,
    objects: Vec<Placement>,
    obstacles: Vec<Obstacle>,
    draped_resources: BTreeSet<Arc<str>>,
    polygons: Vec<DrapedPolygon>,
    polygon_exclusions: Vec<[f64; 4]>,
    triangles: Vec<[Vertex; 3]>,
    bins: Vec<Vec<u32>>,
}
impl Tile {
    fn object_bytes(&self) -> usize {
        let mut geometry = BTreeSet::new();
        let resources = self
            .objects
            .iter()
            .map(|placement| placement.resource.as_ref())
            .collect::<BTreeSet<_>>();
        self.polygon_exclusions.capacity() * std::mem::size_of::<[f64; 4]>()
            + self.polygons.capacity() * std::mem::size_of::<DrapedPolygon>()
            + self
                .polygons
                .iter()
                .map(|polygon| {
                    polygon.resource.len()
                        + polygon.airport.as_ref().map_or(0, |airport| airport.len())
                        + polygon.rings.capacity() * std::mem::size_of::<Vec<[f64; 4]>>()
                        + polygon
                            .rings
                            .iter()
                            .map(|ring| ring.capacity() * std::mem::size_of::<[f64; 4]>())
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self.objects.capacity() * std::mem::size_of::<Placement>()
            + self.obstacles.capacity() * std::mem::size_of::<Obstacle>()
            + self
                .draped_resources
                .iter()
                .map(|resource| resource.len() + std::mem::size_of::<Arc<str>>())
                .sum::<usize>()
            + resources
                .iter()
                .map(|resource| resource.len())
                .sum::<usize>()
            + self
                .obstacles
                .iter()
                .filter(|obstacle| geometry.insert(Arc::as_ptr(&obstacle.geometry) as usize))
                .map(|obstacle| {
                    obstacle.geometry.capacity() * std::mem::size_of::<object::Bounds>()
                })
                .sum::<usize>()
    }
    fn mesh_height(&self, coordinate: [f64; 2]) -> Option<f64> {
        let point = Waypoint {
            latitude: coordinate[0],
            longitude: coordinate[1],
            altitude_feet: 0.0,
        };
        let cell = self.cell([coordinate[1], coordinate[0]]);
        self.bins[cell[1] * GRID + cell[0]]
            .iter()
            .filter_map(|index| {
                let triangle = &self.triangles[*index as usize];
                let weights = barycentric(triangle, point)?;
                (weights.iter().all(|weight| *weight >= -1e-9)).then(|| {
                    triangle
                        .iter()
                        .map(|vertex| vertex.meters)
                        .fold(f64::NEG_INFINITY, f64::max)
                })
            })
            .reduce(f64::max)
    }
    #[cfg(test)]
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::load_with(path, None)
    }
    fn load_with(path: &Path, library: Option<&library::Library>) -> Result<Self, String> {
        let mut tile = dsf::parse(&file_bytes(path)?)?;
        let pack = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or("Scenery tile has no package root")?;
        let mut loaded = BTreeMap::new();
        let mut geometry_bytes = 0;
        for resource in &tile.draped_resources {
            if !library.is_some_and(|library| library.ground_only(pack, resource).unwrap_or(false))
            {
                tile.unresolved_obstacles = true;
            }
        }
        for placement in &tile.objects {
            if !loaded.contains_key(&placement.resource) {
                if loaded.len() >= 256 {
                    tile.unresolved_obstacles = true;
                    continue;
                }
                let relative = Path::new(placement.resource.trim_start_matches('/'));
                let geometry = if relative.components().all(|part| {
                    matches!(
                        part,
                        std::path::Component::Normal(_) | std::path::Component::CurDir
                    )
                }) {
                    let resources = library.map_or_else(
                        || Ok(vec![pack.join(relative)]),
                        |library| library.resources(pack, &placement.resource),
                    );
                    resources
                        .and_then(|resources| {
                            let mut geometry = Vec::new();
                            for resource in resources {
                                let part = object::load(&resource)?;
                                if (geometry.len() + part.len())
                                    * std::mem::size_of::<object::Bounds>()
                                    > 32 * 1024 * 1024
                                {
                                    return Err("Object variants exceed 32 MiB".into());
                                }
                                geometry.extend(part);
                            }
                            Ok(geometry)
                        })
                        .and_then(|geometry| {
                            let bytes = geometry.capacity() * std::mem::size_of::<object::Bounds>();
                            if geometry_bytes + bytes > 32 * 1024 * 1024 {
                                return Err("Tile object geometry exceeds 32 MiB".into());
                            }
                            geometry_bytes += bytes;
                            Ok(Arc::new(geometry))
                        })
                } else {
                    Err("Invalid scenery object resource path".into())
                };
                loaded.insert(placement.resource.clone(), geometry);
            }
            match loaded.get(&placement.resource) {
                Some(Ok(geometry)) => tile.obstacles.push(Obstacle {
                    placement: placement.clone(),
                    geometry: geometry.clone(),
                }),
                _ => tile.unresolved_obstacles = true,
            }
        }
        Ok(tile)
    }

    fn index(&mut self) -> Result<(), String> {
        self.bins = vec![Vec::new(); GRID * GRID];
        let mut references = 0usize;
        for (index, triangle) in self.triangles.iter().enumerate() {
            let low = [
                triangle
                    .iter()
                    .map(|p| p.longitude)
                    .fold(f64::INFINITY, f64::min),
                triangle
                    .iter()
                    .map(|p| p.latitude)
                    .fold(f64::INFINITY, f64::min),
            ];
            let high = [
                triangle
                    .iter()
                    .map(|p| p.longitude)
                    .fold(f64::NEG_INFINITY, f64::max),
                triangle
                    .iter()
                    .map(|p| p.latitude)
                    .fold(f64::NEG_INFINITY, f64::max),
            ];
            let a = self.cell(low);
            let b = self.cell(high);
            for y in a[1]..=b[1] {
                for x in a[0]..=b[0] {
                    references += 1;
                    if references > 8 * 1024 * 1024 {
                        return Err("DSF mesh index exceeds the tile budget".into());
                    }
                    self.bins[y * GRID + x].push(index as u32);
                }
            }
        }
        Ok(())
    }

    fn cell(&self, coordinate: [f64; 2]) -> [usize; 2] {
        let bin = |value: f64, low: f64, high: f64| {
            ((value - low) / (high - low) * GRID as f64)
                .floor()
                .clamp(0.0, (GRID - 1) as f64) as usize
        };
        [
            bin(coordinate[0], self.bounds[0], self.bounds[1]),
            bin(coordinate[1], self.bounds[2], self.bounds[3]),
        ]
    }

    /// Check the full line against every intersecting physical triangle, leaving gaps unknown.
    pub fn terrain_clear(&self, from: Waypoint, to: Waypoint, margin_feet: f64) -> Option<bool> {
        if !(0.0..=3000.0).contains(&margin_feet)
            || [from, to].iter().any(|p| {
                !p.altitude_feet.is_finite()
                    || !(self.bounds[0]..=self.bounds[1]).contains(&p.longitude)
                    || !(self.bounds[2]..=self.bounds[3]).contains(&p.latitude)
            })
        {
            return None;
        }
        let a = self.cell([
            from.longitude.min(to.longitude),
            from.latitude.min(to.latitude),
        ]);
        let b = self.cell([
            from.longitude.max(to.longitude),
            from.latitude.max(to.latitude),
        ]);
        let mut candidates: BTreeSet<u32> = BTreeSet::new();
        for y in a[1]..=b[1] {
            for x in a[0]..=b[0] {
                candidates.extend(self.bins[y * GRID + x].iter().copied());
            }
        }
        let mut covered = Vec::new();
        for index in candidates {
            let triangle = &self.triangles[index as usize];
            let Some(weights) = barycentric(triangle, from) else {
                continue;
            };
            let Some(last) = barycentric(triangle, to) else {
                continue;
            };
            let mut low: f64 = 0.0;
            let mut high: f64 = 1.0;
            for (start, end) in weights.into_iter().zip(last) {
                let change = end - start;
                if change.abs() < 1e-15 {
                    if start < -1e-9 {
                        low = 1.0;
                        high = 0.0;
                        break;
                    }
                } else {
                    let edge = -start / change;
                    if change > 0.0 {
                        low = low.max(edge);
                    } else {
                        high = high.min(edge);
                    }
                }
            }
            if low > high + 1e-10 {
                continue;
            }
            low = low.clamp(0.0, 1.0);
            high = high.clamp(0.0, 1.0);
            if low > high {
                continue;
            }
            for at in [low, high] {
                let ground = weights
                    .iter()
                    .zip(last)
                    .zip(triangle)
                    .map(|((start, end), p)| (start + (end - start) * at) * p.meters)
                    .sum::<f64>()
                    / 0.3048;
                let aircraft = from.altitude_feet + (to.altitude_feet - from.altitude_feet) * at;
                if aircraft + 1e-6 < ground + margin_feet {
                    return Some(false);
                }
            }
            covered.push((low, high));
        }
        covered.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut through = 0.0;
        for (low, high) in covered {
            if low > through + 1e-8 {
                return None;
            }
            through = high.max(through);
        }
        (through >= 1.0 - 1e-8).then_some(true)
    }
}

fn barycentric(triangle: &[Vertex; 3], point: Waypoint) -> Option<[f64; 3]> {
    let [a, b, c] = triangle;
    let determinant = (b.latitude - c.latitude) * (a.longitude - c.longitude)
        + (c.longitude - b.longitude) * (a.latitude - c.latitude);
    if determinant.abs() < 1e-20 {
        return None;
    }
    let first = ((b.latitude - c.latitude) * (point.longitude - c.longitude)
        + (c.longitude - b.longitude) * (point.latitude - c.latitude))
        / determinant;
    let second = ((c.latitude - a.latitude) * (point.longitude - c.longitude)
        + (a.longitude - c.longitude) * (point.latitude - c.latitude))
        / determinant;
    Some([first, second, 1.0 - first - second])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(name: &[u8; 4], payload: Vec<u8>) -> Vec<u8> {
        let mut result = name.iter().rev().copied().collect::<Vec<_>>();
        result.extend(((payload.len() + 8) as u32).to_le_bytes());
        result.extend(payload);
        result
    }
    fn append(bytes: &mut Vec<u8>, id: &[u8; 4], more: Vec<u8>) {
        let key = id.iter().rev().copied().collect::<Vec<_>>();
        let at = bytes.windows(4).position(|part| part == key).unwrap();
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        bytes.splice(at + size..at + size, more.iter().copied());
        bytes[at + 4..at + 8].copy_from_slice(&((size + more.len()) as u32).to_le_bytes());
    }

    fn mesh_tile(west: i32, south: i32, meters: f32, overlay: bool, obstacles: bool) -> Vec<u8> {
        let props = [
            "sim/west".into(),
            west.to_string(),
            "sim/east".into(),
            (west + 1).to_string(),
            "sim/south".into(),
            south.to_string(),
            "sim/north".into(),
            (south + 1).to_string(),
            "sim/overlay".into(),
            if overlay { "1".into() } else { "0".into() },
        ]
        .into_iter()
        .flat_map(|s: String| s.into_bytes().into_iter().chain([0]))
        .collect::<Vec<_>>();
        let mut pool = 4u32.to_le_bytes().to_vec();
        pool.push(5);
        for values in [
            [0, 65535, 65535, 0],
            [0, 0, 65535, 65535],
            [0, 0, 0, 0],
            [0, 0, 0, 0],
            [0, 0, 0, 0],
        ] {
            pool.push(0);
            for value in values {
                pool.extend((value as u16).to_le_bytes());
            }
        }
        let mut scale = Vec::new();
        for (factor, offset) in [
            (1.0, west as f32),
            (1.0, south as f32),
            (1.0, meters),
            (0.0, 0.0),
            (0.0, 0.0),
        ] {
            scale.extend(f32::to_le_bytes(factor));
            scale.extend(offset.to_le_bytes());
        }
        let mut commands = vec![
            1, 0, 0, 3, 0, 17, 1, 25, 0, 0, 3, 0, 23, 3, 0, 0, 2, 0, 3, 0,
        ];
        if obstacles {
            // One nested-polygon range has an initial index followed by one range endpoint.
            commands.extend([15, 0, 0, 1, 0, 0, 3, 0]);
            commands.extend([32, 2, 0, 0]);
        }
        let mut result = b"XPLNEDSF".to_vec();
        result.extend(1u32.to_le_bytes());
        result.extend(atom(b"HEAD", atom(b"PROP", props)));
        result.extend(atom(b"DEFN", atom(b"TERT", b"terrain\0".to_vec())));
        let mut geodata = atom(b"POOL", pool);
        geodata.extend(atom(b"SCAL", scale));
        result.extend(atom(b"GEOD", geodata));
        result.extend(atom(b"CMDS", commands));
        result.extend([0; 16]);
        result
    }

    #[test]
    fn surface_tiles_follow_the_airport_graph_across_degree_boundaries() {
        let mut airport = openatc_core::airport::Airport {
            reference_latitude: 0.5,
            reference_longitude: 0.5,
            ..Default::default()
        };
        assert_eq!(airport_tiles(&airport).unwrap(), vec![(0, 0)]);
        airport.nodes.push(openatc_core::airport::TaxiNode {
            id: 1,
            point: openatc_core::state::Point {
                north: 66792.0,
                ..Default::default()
            },
        });
        assert_eq!(airport_tiles(&airport).unwrap(), vec![(0, 0), (1, 0)]);
        airport.nodes[0].point.east = f64::NAN;
        assert!(airport_tiles(&airport).is_err());
        airport.nodes.clear();
        airport.reference_longitude = 179.999;
        assert!(airport_tiles(&airport).is_err());
    }

    #[test]
    fn supplemental_pavement_obeys_owner_filters_uv_layout_and_higher_exclusions() {
        fn property(bytes: &mut Vec<u8>, text: &[u8]) {
            let at = bytes.windows(4).position(|part| part == b"DAEH").unwrap();
            let old = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
            append(bytes, b"PROP", text.to_vec());
            bytes[at + 4..at + 8].copy_from_slice(&(old + text.len() as u32).to_le_bytes());
        }
        fn pavement(airport: &str, uv: bool) -> Vec<u8> {
            let mut bytes = mesh_tile(0, 0, 0.0, true, false);
            property(
                &mut bytes,
                format!("sim/filter/aptid\0{airport}\0").as_bytes(),
            );
            append(&mut bytes, b"DEFN", atom(b"POLY", b"paved.pol\0".to_vec()));
            let mut pool = 4u32.to_le_bytes().to_vec();
            let depth = if uv { 4 } else { 2 };
            pool.push(depth);
            for values in [
                [0u16, 65535, 65535, 0],
                [0u16, 0, 65535, 65535],
                [0; 4],
                [0; 4],
            ]
            .iter()
            .take(depth as usize)
            {
                pool.push(0);
                for value in values {
                    pool.extend(value.to_le_bytes());
                }
            }
            let mut scales = Vec::new();
            for _ in 0..depth {
                scales.extend(1.0f32.to_le_bytes());
                scales.extend(0.0f32.to_le_bytes());
            }
            let mut geodata = atom(b"POOL", pool);
            geodata.extend(atom(b"SCAL", scales));
            append(&mut bytes, b"GEOD", geodata);
            let parameter = if uv { u16::MAX } else { 0 };
            let mut commands = vec![1, 1, 0, 3, 0, 32, 6, 1, 0, 0, 0, 0, 0, 13];
            commands.extend(parameter.to_le_bytes());
            commands.extend([0, 0, 4, 0]);
            append(&mut bytes, b"CMDS", commands);
            bytes
        }
        let root = std::env::temp_dir().join(format!(
            "openatc-pavement-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = root.join("Global Scenery/X-Plane 12 Global Scenery");
        let relative = "Earth nav data/+00+000/+00+000.dsf";
        std::fs::create_dir_all(base.join("Earth nav data/+00+000")).unwrap();
        std::fs::write(
            base.join("paved.pol"),
            "A\n850\nDRAPED_POLYGON\nSURFACE concrete\n",
        )
        .unwrap();
        let airport = openatc_core::airport::Airport {
            icao: "TEST".into(),
            source: base
                .join("Earth nav data/apt.dat")
                .to_string_lossy()
                .into_owned(),
            reference_latitude: 0.5,
            reference_longitude: 0.5,
            ..Default::default()
        };
        for uv in [false, true] {
            std::fs::write(base.join(relative), pavement("TEST", uv)).unwrap();
            let mut selected = airport.clone();
            assert_eq!(
                Terrain::new(&root)
                    .unwrap()
                    .supplement_airport(&mut selected)
                    .unwrap(),
                1
            );
            assert!(openatc_core::airport::on_pavement(
                &selected,
                Default::default()
            ));
        }
        std::fs::write(base.join(relative), pavement("OTHER", false)).unwrap();
        assert_eq!(
            Terrain::new(&root)
                .unwrap()
                .supplement_airport(&mut airport.clone())
                .unwrap(),
            0
        );
        std::fs::write(base.join(relative), pavement("TEST", false)).unwrap();
        let mut different_owner = airport.clone();
        different_owner.source = root
            .join("Other/Earth nav data/apt.dat")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            Terrain::new(&root)
                .unwrap()
                .supplement_airport(&mut different_owner)
                .unwrap(),
            0
        );
        let higher = root.join("Custom Scenery/Higher");
        std::fs::create_dir_all(higher.join("Earth nav data/+00+000")).unwrap();
        std::fs::write(
            root.join("Custom Scenery/scenery_packs.ini"),
            "SCENERY_PACK Custom Scenery/Higher/\n",
        )
        .unwrap();
        let mut excluded = mesh_tile(0, 0, 0.0, true, false);
        property(&mut excluded, b"sim/exclude_pol\00/0/1/1;0,0,1,0,1,1,0,1\0");
        std::fs::write(higher.join(relative), excluded).unwrap();
        assert_eq!(
            Terrain::new(&root)
                .unwrap()
                .supplement_airport(&mut airport.clone())
                .unwrap(),
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires installed airport scenery"]
    fn installed_memphis_pavement_covers_passenger_cargo_and_ga_stands() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let mut airport = openatc_core::airport::load_airport_from_simulator(
            &root,
            "KMEM",
            &|path| Path::new(path).is_file(),
            &|path| std::fs::read_to_string(path).map_err(|error| error.to_string()),
        )
        .unwrap();
        let terrain = Terrain::new(Path::new(&root)).unwrap();
        let before = airport
            .parking
            .iter()
            .filter(|stand| openatc_core::airport::on_pavement(&airport, stand.point))
            .count();
        let added = terrain.supplement_airport(&mut airport).unwrap();
        let after = airport
            .parking
            .iter()
            .filter(|stand| openatc_core::airport::on_pavement(&airport, stand.point))
            .count();
        eprintln!("KMEM: {added} physical polygons; covered stands {before} -> {after}");
        assert!(added > 0 && after > before);
        for prefix in ["Gate A19", "Cargo South", "Signature Flight Support"] {
            assert!(
                airport
                    .parking
                    .iter()
                    .filter(|stand| stand.name.starts_with(prefix))
                    .any(|stand| openatc_core::airport::on_pavement(&airport, stand.point)),
                "No physical coverage for {prefix}"
            );
        }
    }

    #[test]
    #[ignore = "requires installed airport scenery"]
    fn installed_memphis_draped_surfaces_retain_pavement_geometry() {
        let root = PathBuf::from(std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap());
        let pack = root.join("Global Scenery/Global Airports");
        let tile = Tile::load(&pack.join("Earth nav data/+30-090/+35-090.dsf")).unwrap();
        let terrain = Terrain::new(&root).unwrap();
        let mut resources = BTreeMap::new();
        let mut hard = 0;
        for polygon in &tile.polygons {
            if polygon.airport.as_deref() != Some("KMEM") {
                continue;
            }
            if !polygon.rings.iter().flatten().any(|point| {
                (35.0..35.12).contains(&point[1]) && (-90.06..-89.9).contains(&point[0])
            }) {
                continue;
            }
            let paved = *resources
                .entry(polygon.resource.clone())
                .or_insert_with(|| {
                    terrain
                        .library
                        .paved_surface(&pack, &polygon.resource)
                        .unwrap_or(false)
                });
            if paved {
                hard += 1;
            }
            eprintln!(
                "{}: airport={:?}, hard={paved}, depth={}, parameter={}, rings={}, points={}",
                polygon.resource,
                polygon.airport,
                polygon.depth,
                polygon.parameter,
                polygon.rings.len(),
                polygon.rings.iter().map(Vec::len).sum::<usize>()
            );
        }
        assert!(hard > 0, "No verified physical draped pavement at Memphis");
    }

    #[test]
    fn draped_resource_names_require_actual_ground_only_library_variants() {
        let root = std::env::temp_dir().join(format!(
            "openatc-draped-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pack = root.join("Global Scenery/X-Plane 12 Global Scenery");
        let path = pack.join("Earth nav data/+00+000/+00+000.dsf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = mesh_tile(0, 0, 100.0, false, true);
        append(&mut bytes, b"DEFN", atom(b"POLY", b"paint.pol\0".to_vec()));
        std::fs::write(path, bytes).unwrap();
        std::fs::write(
            pack.join("paint.pol"),
            "A\n850\nDRAPED_POLYGON\nTEXTURE paint.png\n",
        )
        .unwrap();
        let from = Waypoint {
            latitude: 0.5,
            longitude: 0.1,
            altitude_feet: 500.0,
        };
        let to = Waypoint {
            longitude: 0.9,
            ..from
        };
        let mut terrain = Terrain::new(&root).unwrap();
        assert!(terrain.terrain_clear(from, to, 0.0).unwrap());
        assert!(
            !terrain.has_unresolved_obstacles(),
            "Paint does not create a vertical obstacle"
        );
        let library = root.join("Custom Scenery/Redirect");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(
            root.join("Custom Scenery/scenery_packs.ini"),
            "SCENERY_PACK Custom Scenery/Redirect/\n",
        )
        .unwrap();
        std::fs::write(library.join("library.txt"), "A\n1200\nLIBRARY\nEXPORT_SEASON spr,sum,fal paint.pol plain.pol\nEXPORT_SEASON win paint.pol replacement.obj\n").unwrap();
        std::fs::write(library.join("plain.pol"), "I\n850\nDRAPED_POLYGON\n").unwrap();
        std::fs::write(
            library.join("replacement.obj"),
            "I\n800\nOBJ\nPOINT_COUNTS 0 0 0 0\n",
        )
        .unwrap();
        let mut redirected = Terrain::new(&root).unwrap();
        assert!(redirected.terrain_clear(from, to, 0.0).unwrap());
        assert!(
            redirected.has_unresolved_obstacles(),
            "The virtual extension cannot hide another resource type"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_local_object_placement_checks_height_rotation_and_aircraft_width() {
        let root = std::env::temp_dir().join(format!(
            "openatc-object-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let pack = root.join("Global Scenery/X-Plane 12 Global Scenery");
        let path = pack.join("Earth nav data/+00+000/+00+000.dsf");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = mesh_tile(0, 0, 100.0, false, false);
        append(&mut bytes, b"DEFN", atom(b"OBJT", b"tower.obj\0".to_vec()));
        let mut pool = 1u32.to_le_bytes().to_vec();
        pool.push(3);
        for value in [32768u16, 32768, 90] {
            pool.push(0);
            pool.extend(value.to_le_bytes());
        }
        let mut scale = Vec::new();
        for factor in [1.0f32, 1.0, 0.0] {
            scale.extend(factor.to_le_bytes());
            scale.extend(0.0f32.to_le_bytes());
        }
        let mut geodata = atom(b"POOL", pool);
        geodata.extend(atom(b"SCAL", scale));
        append(&mut bytes, b"GEOD", geodata);
        append(&mut bytes, b"CMDS", vec![1, 1, 0, 3, 0, 7, 0, 0]);
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(pack.join("tower.obj"), "I\n800\nOBJ\nPOINT_COUNTS 3 0 0 3\nVT -2 0 -20 0 1 0 0 0\nVT 2 12 -20 0 1 0 0 0\nVT 0 0 20 0 1 0 0 0\nIDX 0\nIDX 1\nIDX 2\nTRIS 0 3\n").unwrap();
        let mut terrain = Terrain::new(&root).unwrap();
        let from = Waypoint {
            latitude: 32768.0 / 65535.0,
            longitude: 0.4995,
            altitude_feet: 360.0,
        };
        let to = Waypoint {
            longitude: 0.5005,
            ..from
        };
        assert!(terrain.terrain_clear(from, to, 0.0).unwrap());
        assert_eq!(terrain.tile(0, 0).unwrap()[0].obstacles.len(), 1);
        assert!(!terrain.obstacles_clear(from, to, 0.0).unwrap());
        assert!(
            terrain
                .obstacles_clear(
                    Waypoint {
                        altitude_feet: 400.0,
                        ..from
                    },
                    Waypoint {
                        altitude_feet: 400.0,
                        ..to
                    },
                    0.0
                )
                .unwrap()
        );
        let shifted = |p: Waypoint| Waypoint {
            latitude: p.latitude + 12.0 / 111320.0,
            ..p
        };
        assert!(
            terrain
                .obstacles_clear(shifted(from), shifted(to), 0.0)
                .unwrap()
        );
        assert!(
            !terrain
                .obstacles_clear(shifted(from), shifted(to), 15.0)
                .unwrap()
        );
        assert!(
            terrain.has_unresolved_obstacles(),
            "Local geometry cannot stand in for full obstacle coverage"
        );
        let library_pack = root.join("Custom Scenery/Variants");
        std::fs::create_dir_all(&library_pack).unwrap();
        std::fs::write(
            root.join("Custom Scenery/scenery_packs.ini"),
            "SCENERY_PACK Custom Scenery/Variants/\n",
        )
        .unwrap();
        std::fs::write(library_pack.join("library.txt"), "I\n1200\nLIBRARY\nEXPORT_SEASON win tower.obj tall.obj\nEXPORT_RATIO_SEASON spr,sum,fal 1 tower.obj tall.obj\n").unwrap();
        let tower = std::fs::read_to_string(pack.join("tower.obj")).unwrap();
        std::fs::write(
            library_pack.join("tall.obj"),
            tower.replace("2 12 -20", "2 35 -20"),
        )
        .unwrap();
        let mut variants = Terrain::new(&root).unwrap();
        assert!(
            !variants
                .obstacles_clear(
                    Waypoint {
                        altitude_feet: 400.0,
                        ..from
                    },
                    Waypoint {
                        altitude_feet: 400.0,
                        ..to
                    },
                    0.0
                )
                .unwrap(),
            "The taller library variant must not be replaced by the smaller local object"
        );
        assert!(
            variants
                .obstacles_clear(
                    Waypoint {
                        altitude_feet: 450.0,
                        ..from
                    },
                    Waypoint {
                        altitude_feet: 450.0,
                        ..to
                    },
                    0.0
                )
                .unwrap()
        );
        let mut invalid = bytes.clone();
        let command = invalid.len() - 18;
        invalid[command..command + 2].copy_from_slice(&9u16.to_le_bytes());
        assert!(dsf::parse(&invalid).is_err());
        std::fs::write(
            pack.join("tower.obj"),
            "I\n800\nOBJ\nPOINT_COUNTS 0 0 0 0\nANIM_begin\n",
        )
        .unwrap();
        let unresolved = Tile::load(&path).unwrap();
        assert!(unresolved.obstacles.is_empty());
        assert!(unresolved.unresolved_obstacles);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dsf_record_bounds_and_nested_polygon_ranges_are_checked_before_mesh_release() {
        let bytes = mesh_tile(0, 0, 200.0, false, true);
        let tile = dsf::parse(&bytes).unwrap();
        assert!(tile.unresolved_obstacles);
        assert_eq!(tile.triangles.len(), 2);
        let a = Waypoint {
            latitude: 0.5,
            longitude: 0.1,
            altitude_feet: 1000.0,
        };
        let b = Waypoint {
            longitude: 0.9,
            ..a
        };
        assert_eq!(tile.terrain_clear(a, b, 100.0), Some(true));
        for length in [0, 8, 12, bytes.len() - 1, bytes.len() / 2] {
            assert!(dsf::parse(&bytes[..length]).is_err());
        }
        let mut bad = bytes.clone();
        bad[16..20].copy_from_slice(&7u32.to_le_bytes());
        assert!(dsf::parse(&bad).is_err());
        let mut bad = bytes;
        let commands = bad.windows(4).position(|s| s == b"SDMC").unwrap() + 8;
        bad[commands] = 200;
        assert!(dsf::parse(&bad).is_err());
    }

    #[test]
    fn enabled_stack_preserves_overlay_heights_base_priority_boundaries_and_unknowns() {
        let root = std::env::temp_dir().join(format!(
            "openatc-mesh-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |base: &str, lat: i32, lon: i32, height, overlay, obstacles| {
            let path = root
                .join(base)
                .join("Earth nav data")
                .join(format!(
                    "{:+03}{:+04}",
                    lat.div_euclid(10) * 10,
                    lon.div_euclid(10) * 10
                ))
                .join(format!("{lat:+03}{lon:+04}.dsf"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, mesh_tile(lon, lat, height, overlay, obstacles)).unwrap();
        };
        write(
            "Global Scenery/X-Plane 12 Global Scenery",
            0,
            0,
            100.0,
            false,
            false,
        );
        write("Custom Scenery/Disabled", 0, 0, 1000.0, false, false);
        write("Custom Scenery/Overlay", 0, 0, 200.0, true, true);
        write("Custom Scenery/Base", 0, 0, 50.0, false, false);
        let config = root.join("Custom Scenery/scenery_packs.ini");
        std::fs::write(&config,"SCENERY_PACK_DISABLED Custom Scenery/Disabled/\nSCENERY_PACK Custom Scenery/Overlay/\nSCENERY_PACK Custom Scenery/Base/\n").unwrap();
        let a = Waypoint {
            latitude: 0.5,
            longitude: 0.1,
            altitude_feet: 600.0,
        };
        let b = Waypoint {
            longitude: 0.9,
            ..a
        };
        let mut terrain = Terrain::new(&root).unwrap();
        assert!(!terrain.terrain_clear(a, b, 0.0).unwrap());
        assert!(terrain.has_unresolved_obstacles());
        terrain.begin_check();
        assert!(!terrain.has_unresolved_obstacles());
        assert!(
            terrain
                .terrain_clear(
                    Waypoint {
                        altitude_feet: 1000.0,
                        ..a
                    },
                    Waypoint {
                        altitude_feet: 1000.0,
                        ..b
                    },
                    0.0
                )
                .unwrap()
        );
        assert!(terrain.has_unresolved_obstacles());
        std::fs::write(
            &config,
            "SCENERY_PACK_DISABLED Custom Scenery/Disabled/\nSCENERY_PACK Custom Scenery/Base/\n",
        )
        .unwrap();
        let mut terrain = Terrain::new(&root).unwrap();
        assert!(
            terrain
                .terrain_clear(
                    Waypoint {
                        altitude_feet: 300.0,
                        ..a
                    },
                    Waypoint {
                        altitude_feet: 300.0,
                        ..b
                    },
                    0.0
                )
                .unwrap()
        );
        assert!(!terrain.has_unresolved_obstacles());
        write(
            "Global Scenery/X-Plane 12 Global Scenery",
            0,
            1,
            50.0,
            false,
            false,
        );
        assert!(
            terrain
                .terrain_clear(
                    Waypoint {
                        longitude: 0.9999,
                        ..a
                    },
                    Waypoint {
                        longitude: 1.0001,
                        ..b
                    },
                    0.0
                )
                .unwrap()
        );
        assert!(
            terrain
                .terrain_clear(
                    Waypoint { latitude: 2.0, ..a },
                    Waypoint { latitude: 2.0, ..b },
                    0.0
                )
                .is_err()
        );
        write(
            "Global Scenery/X-Plane 12 Global Scenery",
            -1,
            -1,
            50.0,
            false,
            false,
        );
        assert!(
            terrain
                .terrain_clear(
                    Waypoint {
                        latitude: -0.5,
                        longitude: -0.9,
                        ..a
                    },
                    Waypoint {
                        latitude: -0.5,
                        longitude: -0.1,
                        ..b
                    },
                    0.0
                )
                .unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn mesh_checks_a_ridge_between_endpoints_and_does_not_bridge_unknown_gaps() {
        let vertex = |longitude, latitude, meters| Vertex {
            longitude,
            latitude,
            meters,
        };
        let mut tile = Tile {
            bounds: [0.0, 1.0, 0.0, 1.0],
            overlay: false,
            unresolved_obstacles: false,
            objects: Vec::new(),
            obstacles: Vec::new(),
            draped_resources: BTreeSet::new(),
            polygons: Vec::new(),
            polygon_exclusions: Vec::new(),
            triangles: vec![
                [
                    vertex(0.0, 0.0, 0.0),
                    vertex(0.5, 0.0, 200.0),
                    vertex(0.5, 1.0, 200.0),
                ],
                [
                    vertex(0.0, 0.0, 0.0),
                    vertex(0.5, 1.0, 200.0),
                    vertex(0.0, 1.0, 0.0),
                ],
                [
                    vertex(0.5, 0.0, 200.0),
                    vertex(1.0, 0.0, 0.0),
                    vertex(1.0, 1.0, 0.0),
                ],
                [
                    vertex(0.5, 0.0, 200.0),
                    vertex(1.0, 1.0, 0.0),
                    vertex(0.5, 1.0, 200.0),
                ],
            ],
            bins: Vec::new(),
        };
        tile.index().unwrap();
        let point = |longitude, altitude_feet| Waypoint {
            latitude: 0.5,
            longitude,
            altitude_feet,
        };
        assert_eq!(
            tile.terrain_clear(point(0.1, 300.0), point(0.9, 300.0), 0.0),
            Some(false)
        );
        assert_eq!(
            tile.terrain_clear(point(0.1, 1000.0), point(0.9, 1000.0), 100.0),
            Some(true)
        );
        assert_eq!(
            tile.terrain_clear(point(0.1, 700.0), point(0.9, 700.0), 100.0),
            Some(false)
        );
        tile.triangles.truncate(2);
        tile.index().unwrap();
        assert_eq!(
            tile.terrain_clear(point(0.1, 1000.0), point(0.9, 1000.0), 0.0),
            None
        );
        assert_eq!(
            tile.terrain_clear(point(0.1, 1000.0), point(1.1, 1000.0), 0.0),
            None
        );
        assert_eq!(
            tile.terrain_clear(point(0.1, f64::NAN), point(0.2, 1000.0), 0.0),
            None
        );
    }

    #[test]
    #[ignore = "requires installed licensed X-Plane scenery"]
    fn installed_enabled_nice_stack_retains_unknown_obstacles() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let mut terrain = Terrain::new(Path::new(&root)).unwrap();
        let a = Waypoint {
            latitude: 43.656,
            longitude: 7.2,
            altitude_feet: 10000.0,
        };
        let b = Waypoint {
            longitude: 7.21,
            ..a
        };
        assert!(terrain.terrain_clear(a, b, 1000.0).unwrap());
        assert!(terrain.has_unresolved_obstacles());
        assert!(
            !terrain
                .terrain_clear(
                    Waypoint {
                        altitude_feet: -100.0,
                        ..a
                    },
                    Waypoint {
                        altitude_feet: -100.0,
                        ..b
                    },
                    0.0
                )
                .unwrap()
        );
    }

    #[test]
    #[ignore = "requires installed licensed X-Plane scenery"]
    fn installed_nice_mesh_decodes_on_the_planning_worker() {
        let root = std::env::var("OPENATC_TEST_SIMULATOR_ROOT").unwrap();
        let tile =
            Tile::load(&Path::new(&root).join(
                "Global Scenery/X-Plane 12 Global Scenery/Earth nav data/+40+000/+43+007.dsf",
            ))
            .unwrap();
        assert_eq!(tile.bounds, [7.0, 8.0, 43.0, 44.0]);
        assert!(!tile.overlay);
        assert!(!tile.triangles.is_empty());
        let a = Waypoint {
            latitude: 43.656,
            longitude: 7.2,
            altitude_feet: 10000.0,
        };
        let b = Waypoint {
            longitude: 7.21,
            ..a
        };
        assert_eq!(tile.terrain_clear(a, b, 1000.0), Some(true));
        assert_eq!(
            tile.terrain_clear(
                Waypoint {
                    altitude_feet: -100.0,
                    ..a
                },
                Waypoint {
                    altitude_feet: -100.0,
                    ..b
                },
                0.0
            ),
            Some(false)
        );
    }
}
