//!include body.inc globals.inc quat.inc shadow.inc

let c_BodyColorId: u32 = 1u;

[[block]]
struct Storage {
    bodies: array<Body>;
};

[[group(0), binding(2)]] var<storage, read> s_Storage: Storage;

[[group(1), binding(0)]] var t_ColorTable: texture_1d<u32>;

struct Geometry {
    [[location(3)]] pos_scale: vec4<f32>;
    [[location(4)]] orientation: vec4<f32>;
    [[location(6)]] body_and_color_id: vec2<u32>;
};

[[stage(vertex)]]
fn geometry_vs(
    [[location(0)]] vertex: vec4<i32>,
    geo: Geometry,
) -> [[builtin(position)]] vec4<f32> {
    let body_id = i32(geo.body_and_color_id.x);
    let body_pos_scale = s_Storage.bodies[body_id].pos_scale;
    let body_orientation = s_Storage.bodies[body_id].orientation;

    let local = qrot(geo.orientation, vec3<f32>(vertex.xyz)) * geo.pos_scale.w + geo.pos_scale.xyz;
    let world = qrot(body_orientation, local) * body_pos_scale.w + body_pos_scale.xyz;
    return u_Globals.view_proj * vec4<f32>(world, 1.0);
}

struct Varyings {
    [[builtin(position)]] proj_pos: vec4<f32>;
    [[location(0)]] palette_range: vec2<f32>;
    [[location(1)]] position: vec3<f32>;
    [[location(2)]] normal: vec3<f32>;
};

[[stage(vertex)]]
fn color_vs(
    [[location(0)]] vertex: vec4<i32>,
    [[location(1)]] color_index: u32,
    [[location(2)]] normal: vec4<f32>,
    geo: Geometry,
) -> Varyings {
    let body_id = i32(geo.body_and_color_id.x);
    let body_pos_scale = s_Storage.bodies[body_id].pos_scale;
    let body_orientation = s_Storage.bodies[body_id].orientation;

    let local = qrot(geo.orientation, vec3<f32>(vertex.xyz)) * geo.pos_scale.w + geo.pos_scale.xyz;
    let world = qrot(body_orientation, local) * body_pos_scale.w + body_pos_scale.xyz;

    let color_id = select(color_index, geo.body_and_color_id.y, color_index == c_BodyColorId);
    let range = textureLoad(t_ColorTable, i32(color_id), 0).xy;
    let palette_range = vec2<f32>(vec2<u32>(range.x, range.x + (128u >> range.y)));

    let n = normalize(normal.xyz);
    let world_normal = qrot(body_orientation, qrot(geo.orientation, n));
    return Varyings(
        u_Globals.view_proj * vec4<f32>(world, 1.0),
        palette_range,
        world,
        world_normal,
    );
}


[[group(0), binding(1)]] var s_PaletteSampler: sampler;
[[group(1), binding(1)]] var t_Palette: texture_1d<f32>;

[[stage(fragment)]]
fn color_fs(in: Varyings, [[builtin(front_facing)]] is_front: bool) -> [[location(0)]] vec4<f32> {
    let lit_factor = fetch_shadow(in.position);
    let normal = normalize(in.normal) * select(1.0, -1.0, is_front);
    let light = normalize(u_Globals.light_pos.xyz - in.position * u_Globals.light_pos.w);
    let n_dot_l = lit_factor * max(0.0, dot(normal, light));
    let tc_raw = mix(in.palette_range.x, in.palette_range.y, n_dot_l);
    let tc = clamp(tc_raw, in.palette_range.x + 0.5, in.palette_range.y - 0.5) / 256.0;
    return textureSample(t_Palette, s_PaletteSampler, tc);
}
