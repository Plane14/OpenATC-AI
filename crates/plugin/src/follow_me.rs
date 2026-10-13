//! One instanced scenery vehicle; no flight model or traffic slots are claimed.
#![allow(clippy::cast_possible_truncation)]
use openatc_core::{
    escort::Escort,
    state::{Point, State},
};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    ptr,
    time::Instant,
};
use xplane_sys as xp;

pub struct FollowMe {
    route: Escort,
    object: xp::XPLMObjectRef,
    instance: xp::XPLMInstanceRef,
    probe: xp::XPLMProbeRef,
    attempted: bool,
    active: bool,
    elevation: f64,
    last_step: Instant,
    last_pose: Option<(Point, f64)>,
}

impl FollowMe {
    pub fn new() -> Self {
        Self {
            route: Escort::default(),
            object: ptr::null_mut(),
            instance: ptr::null_mut(),
            probe: ptr::null_mut(),
            attempted: false,
            active: false,
            elevation: 0.0,
            last_step: Instant::now(),
            last_pose: None,
        }
    }

    pub fn update(&mut self, enabled: bool, state: &State) {
        if !enabled || !self.route.update(state) {
            self.hide();
            self.route.clear();
            return;
        }
        self.elevation = state.telemetry.altitude_feet * 0.3048;
        if self.object.is_null() && !self.attempted {
            self.attempted = true;
            let mut path: Option<CString> = None;
            unsafe {
                xp::XPLMLookupObjects(
                    c"lib/airport/Common_Elements/Vehicles/airside_operations/followme_car_01.obj"
                        .as_ptr(),
                    state.telemetry.latitude as f32,
                    state.telemetry.longitude as f32,
                    Some(found),
                    (&raw mut path).cast::<c_void>(),
                );
                if let Some(path) = path {
                    self.object = xp::XPLMLoadObject(path.as_ptr());
                }
            }
            if self.object.is_null() {
                crate::debug_log(
                    "OpenATC: follow-me car unavailable in the installed scenery library\n",
                );
            }
        }
        if self.object.is_null() {
            return;
        }
        unsafe {
            if self.probe.is_null() {
                self.probe = xp::XPLMCreateProbe(xp::XPLMProbeType::Y);
            }
        }
        self.active = !self.probe.is_null();
        self.last_pose = None;
    }

    pub fn active(&self) -> bool {
        self.active
    }

    // Called at 20 Hz only while the guide is active; at most one terrain probe per call.
    pub fn step(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_step).as_secs_f64();
        self.last_step = now;
        if !self.active {
            return;
        }
        let Some(pose) = self.route.advance(elapsed) else {
            self.hide();
            return;
        };
        if self.last_pose == Some(pose) {
            return;
        }
        let (point, heading) = pose;
        let latitude = self.route.latitude + point.north / 111_320.0;
        let longitude = self.route.longitude
            + point.east / (111_320.0 * self.route.latitude.to_radians().cos());
        let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
        let mut hit: xp::XPLMProbeInfo_t = unsafe { std::mem::zeroed() };
        hit.structSize = i32::try_from(std::mem::size_of_val(&hit)).expect("SDK struct size");
        unsafe {
            xp::XPLMWorldToLocal(
                latitude,
                longitude,
                self.elevation,
                &raw mut x,
                &raw mut y,
                &raw mut z,
            );
            let result = xp::XPLMProbeTerrainXYZ(
                self.probe,
                x as f32,
                (y + 1000.0) as f32,
                z as f32,
                &raw mut hit,
            );
            if result != xp::XPLMProbeResult::HitTerrain
                || hit.is_wet != 0
                || !hit.locationY.is_finite()
                || hit.normalY < 0.8
            {
                self.hide();
                return;
            }
            if self.instance.is_null() {
                let mut refs = [ptr::null()];
                self.instance = xp::XPLMCreateInstance(self.object, refs.as_mut_ptr());
            }
            if self.instance.is_null() {
                return;
            }
            let radians = heading.to_radians() as f32;
            let slope = |dx: f32, dz: f32| -(hit.normalX * dx + hit.normalZ * dz) / hit.normalY;
            let position = xp::XPLMDrawInfo_t {
                structSize: i32::try_from(std::mem::size_of::<xp::XPLMDrawInfo_t>())
                    .expect("SDK struct size"),
                x: x as f32,
                y: hit.locationY + 0.02,
                z: z as f32,
                heading: heading as f32,
                pitch: slope(radians.sin(), -radians.cos()).atan().to_degrees(),
                roll: -slope(radians.cos(), radians.sin()).atan().to_degrees(),
            };
            xp::XPLMInstanceSetPosition(self.instance, &raw const position, ptr::null());
        }
        self.last_pose = Some(pose);
    }

    fn hide(&mut self) {
        if !self.instance.is_null() {
            unsafe {
                xp::XPLMDestroyInstance(self.instance);
            }
            self.instance = ptr::null_mut();
        }
        self.active = false;
        self.last_pose = None;
    }
}

unsafe extern "C-unwind" fn found(path: *const c_char, reference: *mut c_void) {
    if path.is_null() || reference.is_null() {
        return;
    }
    // XPLMLookupObjects invokes the enumerator before returning.
    let result = unsafe { &mut *reference.cast::<Option<CString>>() };
    if result.is_none() {
        *result = Some(unsafe { CStr::from_ptr(path) }.to_owned());
    }
}

impl Drop for FollowMe {
    fn drop(&mut self) {
        self.hide();
        unsafe {
            if !self.probe.is_null() {
                xp::XPLMDestroyProbe(self.probe);
            }
            if !self.object.is_null() {
                xp::XPLMUnloadObject(self.object);
            }
        }
    }
}
