//! Generate OpenGL 2.1 compatibility bindings.
use std::{env, fs::File, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let destination = Path::new(&env::var("OUT_DIR").unwrap()).join("gl21.rs");
    let mut file = File::create(destination).unwrap();
    gl_generator::Registry::new(
        gl_generator::Api::Gl,
        (2, 1),
        gl_generator::Profile::Compatibility,
        gl_generator::Fallbacks::All,
        [],
    )
    .write_bindings(gl_generator::StructGenerator, &mut file)
    .unwrap();
}
