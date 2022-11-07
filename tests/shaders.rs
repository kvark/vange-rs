fn parse(name: &str, substitutions: &[(&str, String)]) {
    println!("Parsing {}", name);
    let code = vangers::render::make_shader_code(name, substitutions).unwrap();
    naga::front::wgsl::Parser::new().parse(&code).unwrap();
}

#[test]
fn parse_shaders() {
    parse("terrain/ray", &[]);
    parse("terrain/paint", &[]);
    parse("terrain/scatter", &[]);
    parse("terrain/paint", &[]);
    parse("terrain/slice", &[]);
    let voxel_subs = [("morton_tile_size", "1u".to_string())];
    parse("terrain/voxel-bake", &voxel_subs);
    parse("terrain/voxel-draw", &voxel_subs);
}
