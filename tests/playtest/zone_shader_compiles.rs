//! Scripted playtest pinning the shader import contract for issue #18.
//!
//! The original `zone_shader.wgsl` imported
//! `bevy_pbr::forward_io::VertexOutput`, the 3D `forward_io`
//! vertex output. The 2D `Material2d` pipeline passes
//! `bevy_sprite::mesh2d_vertex_output::VertexOutput` into the
//! fragment stage instead. Importing the 3D struct's layout for a
//! function the 2D pipeline feeds with the 2D struct makes naga
//! reject the shader at link time -- the GPU never sees the
//! painted cells even though the ECS mirror side of the brush
//! chain still reports the correct bits.
//!
//! The same wrong import was on the background shader. Its
//! `world_position` swizzle is the same shape in both structs, but
//! the surrounding entry-point type still mismatches, so the 2D
//! pipeline would not link it either. The fix imports the 2D
//! struct on both shaders.
//!
//! ECS-only tests cannot catch this regression because the
//! `IntentGrid` and `ZoneMaterial` data flow is correct even when
//! the shaders fail to link. This test reads the shader files
//! and asserts they import the struct the 2D pipeline actually
//! provides, so a future revert fails CI before a verifier has to
//! eyeball a screenshot.

use std::fs;
use std::path::PathBuf;

fn read_shader(relative_path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("could not read shader at {}: {err}", path.display()))
}

const TWO_D_VERTEX_OUTPUT_IMPORT: &str = "#import bevy_sprite::mesh2d_vertex_output::VertexOutput";
const THREE_D_VERTEX_OUTPUT_IMPORT: &str = "bevy_pbr::forward_io::VertexOutput";

fn assert_imports_2d_vertex_output(path: &str) {
    let source = read_shader(path);
    assert!(
        source.contains(TWO_D_VERTEX_OUTPUT_IMPORT),
        "{path} must import the 2D `VertexOutput` (`bevy_sprite::mesh2d_vertex_output`) \
         because the 2D Material2d pipeline passes that struct into the fragment stage. \
         Without this import the 2D pipeline cannot link the shader and painted cells are \
         never rendered, even though the ECS mirror side of the brush chain still reports the \
         correct bits. Got:\n{source}"
    );
    assert!(
        !source.contains(THREE_D_VERTEX_OUTPUT_IMPORT),
        "{path} must not import the 3D `bevy_pbr::forward_io::VertexOutput`; the 2D pipeline \
         does not provide that struct and the shader will fail to link. Got:\n{source}"
    );
}

#[test]
fn zone_shader_imports_the_2d_vertex_output_not_the_3d_one() {
    assert_imports_2d_vertex_output("assets/shaders/zone_shader.wgsl");
}

#[test]
fn background_shader_imports_the_2d_vertex_output_not_the_3d_one() {
    assert_imports_2d_vertex_output("assets/shaders/background_shader.wgsl");
}
