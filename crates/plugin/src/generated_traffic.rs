//! Lightweight instances. All SDK entry points run on the simulator thread.
use openatc_core::state::GeneratedTraffic;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CString, c_void},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use xplane::data::DataType;
use xplane_sys as xp;

unsafe extern "C-unwind" fn unused_value(_cookie: *mut c_void) -> f32 {
    0.0
}

struct Pending {
    alive: Arc<AtomicBool>,
    result: Arc<Mutex<Option<usize>>>,
}
unsafe extern "C-unwind" fn loaded(object: xp::XPLMObjectRef, cookie: *mut c_void) {
    let pending = unsafe { Box::from_raw(cookie.cast::<Pending>()) };
    if pending.alive.load(Ordering::Acquire) {
        *pending
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(object as usize);
    } else if !object.is_null() {
        unsafe { xp::XPLMUnloadObject(object) };
    }
}
struct Asset {
    alive: Arc<AtomicBool>,
    result: Arc<Mutex<Option<usize>>>,
}
impl Drop for Asset {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        if let Some(object) = *self
            .result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            && object != 0
        {
            unsafe { xp::XPLMUnloadObject(object as xp::XPLMObjectRef) };
        }
    }
}
struct Drawn {
    model: String,
    objects: Vec<std::path::PathBuf>,
    instances: Vec<usize>,
    terrain: Option<Terrain>,
}
struct Terrain {
    x: f64,
    z: f64,
    y: f64,
    normal: [f64; 3],
    coordinate: [f64; 2],
}
/// Owns only OpenATC's objects, never another provider's instances.
pub struct Renderer {
    assets: BTreeMap<std::path::PathBuf, Asset>,
    aircraft: BTreeMap<u32, Drawn>,
    channels: Vec<(CString, openatc_core::traffic::animation::Channel)>,
    owned_refs: Vec<usize>,
    snapshots: Vec<GeneratedTraffic>,
    previous: BTreeMap<u32, GeneratedTraffic>,
    received: std::time::Instant,
    root: std::path::PathBuf,
    source_root: std::path::PathBuf,
    probe: usize,
    mappings: BTreeMap<String, openatc_core::traffic::animation::Channel>,
    paused: bool,
    rendered: Vec<GeneratedTraffic>,
    probe_cursor: u32,
    last_draw: std::time::Instant,
}
impl Default for Renderer {
    fn default() -> Self {
        Self {
            assets: BTreeMap::new(),
            aircraft: BTreeMap::new(),
            channels: Vec::new(),
            owned_refs: Vec::new(),
            snapshots: Vec::new(),
            previous: BTreeMap::new(),
            received: std::time::Instant::now(),
            root: std::path::PathBuf::new(),
            source_root: std::path::PathBuf::new(),
            probe: 0,
            mappings: BTreeMap::new(),
            paused: false,
            rendered: Vec::new(),
            probe_cursor: 0,
            last_draw: std::time::Instant::now(),
        }
    }
}
impl Renderer {
    /// Keep the last two worker snapshots; rendering never scans model folders.
    pub fn receive(
        &mut self,
        frames: &[GeneratedTraffic],
        root: &std::path::Path,
        mappings: &BTreeMap<String, openatc_core::traffic::animation::Channel>,
        paused: bool,
    ) {
        if self.source_root != root || self.mappings != *mappings {
            *self = Self::default();
            self.source_root = root.to_path_buf();
            self.mappings.clone_from(mappings);
        }
        if frames == self.snapshots && paused == self.paused {
            return;
        }
        self.paused = paused;
        self.previous = self
            .rendered
            .iter()
            .cloned()
            .map(|frame| (frame.target.id, frame))
            .collect();
        self.snapshots = frames.iter().take(63).cloned().collect();
        for frame in &mut self.snapshots {
            if let Some(previous) = self.previous.get(&frame.target.id)
                && previous.model == frame.model
            {
                frame.model = previous.model.clone();
            }
        }
        self.received = std::time::Instant::now();
        if !frames.is_empty() && self.root.as_os_str().is_empty() {
            self.root = root.canonicalize().unwrap_or_default();
        }
    }
    /// Interpolate snapshots without inventing progress beyond a clearance limit.
    pub fn step(&mut self) {
        if self.last_draw.elapsed().as_secs_f64() < 0.05 {
            return;
        }
        self.last_draw = std::time::Instant::now();
        let elapsed = self.received.elapsed().as_secs_f64().min(0.5);
        let amount = (elapsed / 0.5).clamp(0.0, 1.0);
        let mut frames = self.snapshots.clone();
        for frame in &mut frames {
            frame.animation.propeller_angle = (frame.animation.propeller_angle
                + frame.animation.propeller_rpm * 6.0 * elapsed)
                .rem_euclid(360.0);
            if let Some(previous) = self
                .previous
                .get(&frame.target.id)
                .filter(|old| old.model.id == frame.model.id)
            {
                let mix = |from: f64, to: f64| from + (to - from) * amount;
                frame.target.latitude = mix(previous.target.latitude, frame.target.latitude);
                let longitude =
                    ((frame.target.longitude - previous.target.longitude + 540.0) % 360.0) - 180.0;
                frame.target.longitude =
                    ((previous.target.longitude + longitude * amount + 540.0) % 360.0) - 180.0;
                frame.target.altitude_feet =
                    mix(previous.target.altitude_feet, frame.target.altitude_feet);
                let heading =
                    ((frame.target.track_degrees - previous.target.track_degrees + 540.0) % 360.0)
                        - 180.0;
                frame.target.track_degrees =
                    (previous.target.track_degrees + heading * amount + 360.0) % 360.0;
                let body = frame
                    .body_heading_degrees
                    .unwrap_or(frame.target.track_degrees);
                let old_body = previous
                    .body_heading_degrees
                    .unwrap_or(previous.target.track_degrees);
                frame.body_heading_degrees = Some(
                    (old_body + openatc_core::navigation::turn(old_body, body) * amount)
                        .rem_euclid(360.0),
                );
                frame.animation.pitch = mix(previous.animation.pitch, frame.animation.pitch);
                frame.animation.bank = mix(previous.animation.bank, frame.animation.bank);
                frame.animation.gear = mix(previous.animation.gear, frame.animation.gear);
                frame.animation.flaps = mix(previous.animation.flaps, frame.animation.flaps);
            }
        }
        if self.paused && !self.rendered.is_empty() && !self.snapshots.is_empty() {
            frames.clone_from(&self.rendered);
        }
        self.rendered.clone_from(&frames);
        let root = self.root.clone();
        if !root.as_os_str().is_empty() || frames.is_empty() {
            self.update(&frames, &root);
        }
    }
    /// Use a faster lightweight render loop only while generated traffic exists.
    pub fn active(&self) -> bool {
        !self.snapshots.is_empty() || !self.aircraft.is_empty()
    }
    pub fn frames(&self) -> &[GeneratedTraffic] {
        &self.rendered
    }
    /// Begin at most one object load per update; existing instances remain usable.
    pub fn update(&mut self, frames: &[GeneratedTraffic], root: &std::path::Path) {
        if !frames.is_empty() && self.channels.is_empty() {
            {
                for (name, channel) in &self.mappings {
                    let Ok(name) = CString::new(name.as_str()) else {
                        continue;
                    };
                    if unsafe { xp::XPLMFindDataRef(name.as_ptr()) }.is_null() {
                        let reference = unsafe {
                            xp::XPLMRegisterDataAccessor(
                                name.as_ptr(),
                                f32::sim_type(),
                                0,
                                None,
                                None,
                                Some(unused_value),
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                None,
                                std::ptr::null_mut(),
                                std::ptr::null_mut(),
                            )
                        };
                        if reference.is_null() {
                            continue;
                        }
                        self.owned_refs.push(reference as usize);
                    }
                    self.channels.push((name, *channel));
                }
            }
        }
        let wanted: BTreeSet<_> = frames
            .iter()
            .take(63)
            .map(|frame| frame.target.id)
            .collect();
        let retired: Vec<_> = self
            .aircraft
            .keys()
            .filter(|id| !wanted.contains(id))
            .copied()
            .collect();
        for id in retired {
            self.retire(id);
        }
        let changed: Vec<_> = frames
            .iter()
            .take(63)
            .filter(|frame| {
                self.aircraft.get(&frame.target.id).is_some_and(|drawn| {
                    drawn.model != frame.model.id || drawn.objects != frame.model.objects
                })
            })
            .map(|frame| frame.target.id)
            .collect();
        for id in changed {
            self.retire(id);
        }
        let wanted_objects: BTreeSet<_> = frames
            .iter()
            .take(63)
            .flat_map(|frame| frame.model.objects.iter())
            .collect();
        self.assets.retain(|path, _| wanted_objects.contains(path));
        let mut requested = false;
        let mut candidates: Vec<_> = frames
            .iter()
            .filter(|frame| {
                frame.target.on_ground
                    && self
                        .aircraft
                        .get(&frame.target.id)
                        .and_then(|drawn| drawn.terrain.as_ref())
                        .is_none_or(|ground| {
                            openatc_core::arrival::distance_nm(
                                ground.coordinate,
                                [frame.target.latitude, frame.target.longitude],
                            ) * 1852.0
                                > 5.0
                        })
            })
            .map(|frame| frame.target.id)
            .collect();
        candidates.sort_unstable();
        if let Some(start) = candidates.iter().position(|id| *id > self.probe_cursor) {
            candidates.rotate_left(start);
        }
        let probe_ids: BTreeSet<_> = candidates
            .into_iter()
            .take(4)
            .inspect(|id| self.probe_cursor = *id)
            .collect();
        for frame in frames.iter().take(63) {
            if frame.target.id == 0
                || frame.model.objects.is_empty()
                || frame.model.objects.len() > 8
                || frame.model.objects.iter().any(|path| {
                    !path.starts_with(root)
                        || path
                            .components()
                            .any(|part| matches!(part, std::path::Component::ParentDir))
                })
                || !(-90.0..=90.0).contains(&frame.target.latitude)
                || !(-180.0..=180.0).contains(&frame.target.longitude)
                || !(-2000.0..=60000.0).contains(&frame.target.altitude_feet)
                || !(0.0..360.0).contains(&frame.target.track_degrees)
                || !frame.animation.pitch.is_finite()
                || !frame.animation.bank.is_finite()
            {
                self.retire(frame.target.id);
                continue;
            }
            for path in &frame.model.objects {
                if self.assets.contains_key(path) || requested || self.assets.len() >= 128 {
                    continue;
                }
                let Ok(name) = CString::new(path.to_string_lossy().as_bytes()) else {
                    continue;
                };
                let result = Arc::new(Mutex::new(None));
                let alive = Arc::new(AtomicBool::new(true));
                let cookie = Box::into_raw(Box::new(Pending {
                    alive: alive.clone(),
                    result: result.clone(),
                }));
                self.assets.insert(path.clone(), Asset { alive, result });
                unsafe { xp::XPLMLoadObjectAsync(name.as_ptr(), Some(loaded), cookie.cast()) };
                requested = true;
            }
            let objects: Option<Vec<usize>> = frame
                .model
                .objects
                .iter()
                .map(|path| {
                    self.assets
                        .get(path)
                        .and_then(|asset| {
                            *asset
                                .result
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                        })
                        .filter(|object| *object != 0)
                })
                .collect();
            let Some(objects) = objects else {
                continue;
            };
            if self
                .aircraft
                .get(&frame.target.id)
                .is_some_and(|drawn| drawn.model != frame.model.id)
            {
                self.retire(frame.target.id);
            }
            if !self.aircraft.contains_key(&frame.target.id) {
                self.aircraft.insert(
                    frame.target.id,
                    Drawn {
                        model: frame.model.id.clone(),
                        objects: frame.model.objects.clone(),
                        instances: Vec::new(),
                        terrain: None,
                    },
                );
            }
            let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
            unsafe {
                xp::XPLMWorldToLocal(
                    frame.target.latitude,
                    frame.target.longitude,
                    frame.target.altitude_feet * 0.3048,
                    &raw mut x,
                    &raw mut y,
                    &raw mut z,
                );
            }
            let drawn = self.aircraft.get_mut(&frame.target.id).unwrap();
            let body_heading = frame
                .body_heading_degrees
                .filter(|heading| (0.0..360.0).contains(heading))
                .unwrap_or(frame.target.track_degrees);
            let (mut pitch, mut roll) = (frame.animation.pitch, frame.animation.bank);
            if frame.target.on_ground {
                let distance = drawn
                    .terrain
                    .as_ref()
                    .map_or(f64::INFINITY, |ground| (ground.x - x).hypot(ground.z - z));
                if distance > 5.0 && probe_ids.contains(&frame.target.id) {
                    if self.probe == 0 {
                        self.probe = unsafe { xp::XPLMCreateProbe(xp::XPLMProbeType::Y) } as usize;
                    }
                    if self.probe != 0 {
                        let mut hit: xp::XPLMProbeInfo_t = unsafe { std::mem::zeroed() };
                        hit.structSize = std::mem::size_of_val(&hit) as i32;
                        let result = unsafe {
                            xp::XPLMProbeTerrainXYZ(
                                self.probe as xp::XPLMProbeRef,
                                x as f32,
                                (y + 1000.0) as f32,
                                z as f32,
                                &raw mut hit,
                            )
                        };
                        if result == xp::XPLMProbeResult::HitTerrain
                            && hit.is_wet == 0
                            && hit.locationY.is_finite()
                            && hit.normalY >= 0.8
                        {
                            drawn.terrain = Some(Terrain {
                                x,
                                z,
                                y: f64::from(hit.locationY),
                                normal: [
                                    f64::from(hit.normalX),
                                    f64::from(hit.normalY),
                                    f64::from(hit.normalZ),
                                ],
                                coordinate: [frame.target.latitude, frame.target.longitude],
                            });
                        } else {
                            drawn.terrain = None;
                        }
                    }
                }
                let Some(ground) = &drawn.terrain else {
                    for instance in drawn.instances.drain(..) {
                        unsafe { xp::XPLMDestroyInstance(instance as xp::XPLMInstanceRef) };
                    }
                    continue;
                };
                if (ground.x - x).hypot(ground.z - z) > 25.0 {
                    self.retire(frame.target.id);
                    continue;
                }
                y = ground.y
                    - (ground.normal[0] * (x - ground.x) + ground.normal[2] * (z - ground.z))
                        / ground.normal[1];
                let radians = body_heading.to_radians();
                let slope = |x: f64, z: f64| {
                    -(ground.normal[0] * x + ground.normal[2] * z) / ground.normal[1]
                };
                pitch += slope(radians.sin(), -radians.cos()).atan().to_degrees();
                roll = -slope(radians.cos(), radians.sin()).atan().to_degrees();
            }
            if drawn.instances.is_empty() {
                for object in objects {
                    let mut refs: Vec<_> = self
                        .channels
                        .iter()
                        .map(|(name, _)| name.as_ptr())
                        .collect();
                    refs.push(std::ptr::null());
                    let instance = unsafe {
                        xp::XPLMCreateInstance(object as xp::XPLMObjectRef, refs.as_mut_ptr())
                    };
                    if instance.is_null() {
                        break;
                    }
                    drawn.instances.push(instance as usize);
                }
                if drawn.instances.len() != frame.model.objects.len() {
                    self.retire(frame.target.id);
                    continue;
                }
            }
            y += frame
                .model
                .vertical_offset
                .filter(|offset| offset.is_finite())
                .unwrap_or(0.0);
            let position = xp::XPLMDrawInfo_t {
                structSize: std::mem::size_of::<xp::XPLMDrawInfo_t>() as i32,
                x: x as f32,
                y: y as f32,
                z: z as f32,
                pitch: pitch as f32,
                heading: body_heading as f32,
                roll: roll as f32,
            };
            let values: Vec<_> = self
                .channels
                .iter()
                .map(|(_, channel)| frame.animation.channel(*channel))
                .collect();
            for instance in &self.aircraft[&frame.target.id].instances {
                unsafe {
                    xp::XPLMInstanceSetPosition(
                        *instance as xp::XPLMInstanceRef,
                        &raw const position,
                        values.as_ptr(),
                    )
                };
            }
        }
    }
    fn retire(&mut self, id: u32) {
        if let Some(drawn) = self.aircraft.remove(&id) {
            for instance in drawn.instances {
                unsafe { xp::XPLMDestroyInstance(instance as xp::XPLMInstanceRef) };
            }
        }
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        let ids: Vec<_> = self.aircraft.keys().copied().collect();
        for id in ids {
            self.retire(id);
        }
        self.assets.clear();
        if self.probe != 0 {
            unsafe { xp::XPLMDestroyProbe(self.probe as xp::XPLMProbeRef) };
        }
        for reference in self.owned_refs.drain(..) {
            unsafe { xp::XPLMUnregisterDataAccessor(reference as xp::XPLMDataRef) };
        }
    }
}
