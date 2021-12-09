fn parse(name: &str) {
    let code = vangers::render::make_shader_code(name).unwrap();
    naga::front::wgsl::Parser::new().parse(&code).unwrap();
}

#[test]
fn parse_shaders() {
    parse("terrain/ray");
    parse("terrain/mip");
    parse("terrain/paint");
}
