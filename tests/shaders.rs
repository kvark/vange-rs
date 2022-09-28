fn parse(name: &str, substitutions: &[(&str, String)]) {
    println!("Parsing {}", name);
    let code = vangers::render::make_shader_code(name, substitutions).unwrap();
    naga::front::wgsl::Parser::new().parse(&code).unwrap();
}

#[test]
fn parse_shaders() {
    parse("terrain/ray", &[]);
    parse("terrain/mip", &[]);
    parse("terrain/paint", &[]);
    parse("terrain/scatter", &[]);
    parse("terrain/paint", &[]);
    parse("terrain/slice", &[]);
    let bake_subs = [
        ("group_w", 1.to_string()),
        ("group_h", 1.to_string()),
        ("group_d", 1.to_string()),
    ];
    parse("terrain/voxel-bake", &bake_subs);
    parse("terrain/voxel-draw", &[]);
}
