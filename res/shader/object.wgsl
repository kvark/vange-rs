//!include body.inc globals.inc quat.inc shadow.inc surface.inc

const c_BodyColorId: u32 = 1u;
const c_WheelColorId: u32 = 3u;

struct Storage {
    bodies: array<Body>,
};

@group(0) @binding(2) var<storage, read> s_Storage: Storage;

// Per-Y water level. Lives on the same bind group as the surface
// uniforms / terrain meta (@group(1)) — these are all bound from the
// terrain pipeline, or from the model-viewer stub.
@group(1) @binding(4) var t_Flood: texture_2d<f32>;

// Object-local resources: per-color lookup, palette texture, sampler.
@group(2) @binding(0) var t_ColorTable: texture_2d<u32>;
@group(2) @binding(1) var t_Palette: texture_2d<f32>;
// Palette sampler comes from globals — this group's binding 2 is the
// `NonFiltering` colour-table sampler used only by the vertex stage.
@group(0) @binding(1) var s_PaletteSampler: sampler;

struct Geometry {
    @location(3) pos_scale: vec4<f32>,
    @location(4) orientation: vec4<f32>,
    @location(6) body_and_color_id: vec2<u32>,
};

struct BodyGeometry {
    orient: vec4<f32>,
    pos: vec3<f32>,
    scale: f32,
};

fn get_body(id: u32) -> BodyGeometry {
    //let pos_scale = s_Storage.bodies[id].pos_scale;
    //let orient = s_Storage.bodies[body_id].orientation;
    let pos_scale = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    let orient = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    return BodyGeometry(
        orient,
        pos_scale.xyz,
        pos_scale.w,
    );
}

@vertex
fn geometry_vs(
    @location(0) vertex: vec4<i32>,
    geo: Geometry,
) -> @builtin(position) vec4<f32> {
    let body = get_body(geo.body_and_color_id.x);

    let local = qrot(geo.orientation, vec3<f32>(vertex.xyz)) * geo.pos_scale.w + geo.pos_scale.xyz;
    let world = qrot(body.orient, local) * body.scale + body.pos;
    return u_Globals.view_proj * vec4<f32>(world, 1.0);
}

struct Varyings {
    @builtin(position) proj_pos: vec4<f32>,
    @location(0) palette_range: vec2<f32>,
    @location(1) position: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) color_id: u32,
};

@vertex
fn color_vs(
    @location(0) vertex: vec4<i32>,
    @location(1) color_index: u32,
    @location(2) normal: vec4<f32>,
    geo: Geometry,
) -> Varyings {
    let body = get_body(geo.body_and_color_id.x);

    let local = qrot(geo.orientation, vec3<f32>(vertex.xyz)) * geo.pos_scale.w + geo.pos_scale.xyz;
    let world = qrot(body.orient, local) * body.scale + body.pos;

    let color_id = select(color_index, geo.body_and_color_id.y, color_index == c_BodyColorId);
    let range = textureLoad(t_ColorTable, vec2<i32>(i32(color_id), 0), 0).xy;
    let palette_range = vec2<f32>(vec2<u32>(range.x, range.x + (128u >> range.y)));

    let n = normalize(normal.xyz);
    let world_normal = qrot(body.orient, qrot(geo.orientation, n));
    return Varyings(
        u_Globals.view_proj * vec4<f32>(world, 1.0),
        palette_range,
        world,
        world_normal,
        color_id,
    );
}

const c_WaterTerrain: u32 = 0u;

// Per-Y flood map sample, scaled into world Z. Mirrors the formula in
// `water.wgsl::main_vs`. Sampling a 1×1 stub texture (model viewer)
// returns 0; a stub `texture_scale.z = 0` then keeps the resulting
// water Z at 0, which makes the fragment-side submersion check a
// no-op for vehicles above ground.
fn flood_z_at(world_y: f32) -> f32 {
    let dim = textureDimensions(t_Flood);
    let section_y = u_Surface.texture_scale.y / f32(dim.x);
    let row_raw = i32(floor(world_y / section_y));
    let row = ((row_raw % i32(dim.x)) + i32(dim.x)) % i32(dim.x);
    let raw = textureLoad(t_Flood, vec2<i32>(row, 0), 0).x;
    return raw * u_Surface.texture_scale.z;
}

// Matches the documented fallback water tint from
// `src/level/mod.rs::load` (`0 => (0.0, 0.0, 200.0), // blue (water)`)
// and the surface tone in `water.wgsl`.
const c_UnderwaterColor = vec3<f32>(0.0, 0.0, 200.0 / 255.0);
// 1 / e-fold depth in world units. ~30 → ~95% blue at 90 units submerged.
const c_UnderwaterDepthFactor: f32 = 1.0 / 30.0;

@fragment
fn color_fs(in: Varyings, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    let lit_factor = fetch_shadow(in.position);
    let normal = normalize(in.normal) * select(-1.0, 1.0, is_front);
    let light = normalize(u_Globals.light_pos.xyz - in.position * u_Globals.light_pos.w);
    let n_dot_l = lit_factor * max(0.0, dot(normal, light));
    let tc_raw = mix(in.palette_range.x, in.palette_range.y, n_dot_l);
    let tc = clamp(tc_raw, in.palette_range.x + 0.5, in.palette_range.y - 0.5) / 256.0;
    var color = textureSample(t_Palette, s_PaletteSampler, vec2<f32>(tc, 0.5));
    if (in.color_id == c_WheelColorId) {
        let wheel_level = n_dot_l * (60.0 / 255.0);
        color = vec4<f32>(vec3<f32>(wheel_level), color.a);
    }

    // Per-fragment underwater tint. All texture fetches happen at
    // uniform top level — only the *blend* is conditional. The cell's
    // low terrain type comes from the shared surface helper; the per-Y
    // water level from the flood map.
    let surface = get_surface(in.position.xy);
    let water_z = flood_z_at(in.position.y);
    let submersion = water_z - in.position.z;
    if (surface.low_type == c_WaterTerrain && submersion > 0.0) {
        let mix_amount = 1.0 - exp(-submersion * c_UnderwaterDepthFactor);
        color = vec4<f32>(mix(color.rgb, c_UnderwaterColor, mix_amount), color.a);
    }
    return color;
}
