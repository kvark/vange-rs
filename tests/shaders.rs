use naga::ShaderStage;

const SHADERS: &[(&str, &[(&str, &str)])] = &[
    ("debug", &[]),
    ("object", &[]),
    ("water", &[]),
    ("terrain/ray", &[]),
    ("terrain/paint", &[]),
    ("terrain/scatter", &[]),
    ("terrain/slice", &[]),
    ("terrain/mesh", &[]),
    ("terrain/voxel-bake", &[("morton_tile_size", "1u")]),
    ("terrain/voxel-draw", &[("morton_tile_size", "1u")]),
];

fn parse(name: &str, substitutions: &[(&str, &str)]) -> (String, naga::Module) {
    println!("Parsing {}", name);
    let subs: Vec<(&str, String)> = substitutions
        .iter()
        .map(|&(k, v)| (k, v.to_string()))
        .collect();
    let code = vangers::render::make_shader_code(name, &subs).unwrap();
    let module = naga::front::wgsl::Frontend::new().parse(&code).unwrap();
    (code, module)
}

#[test]
fn parse_shaders() {
    for &(name, subs) in SHADERS {
        parse(name, subs);
    }
}

/// Names of the structs a module passes between the vertex and fragment
/// stages, which are the ones the interpolation rules apply to. Vertex
/// *inputs* look the same in source but must not carry `@interpolate`
/// at all, so they have to be told apart - hence asking naga rather
/// than pattern-matching every struct in the file.
fn inter_stage_structs(module: &naga::Module) -> Vec<String> {
    let name_of = |handle| module.types[handle].name.clone();
    let mut names = Vec::new();
    for ep in &module.entry_points {
        match ep.stage {
            ShaderStage::Vertex => {
                names.extend(ep.function.result.as_ref().and_then(|r| name_of(r.ty)))
            }
            ShaderStage::Fragment => {
                names.extend(ep.function.arguments.iter().filter_map(|a| name_of(a.ty)))
            }
            _ => {}
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// Members of `struct <name>` in the source, one entry per member line.
fn struct_members<'a>(code: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("struct {} {{", name);
    let Some(start) = code.find(&header) else {
        return Vec::new();
    };
    let body = &code[start + header.len()..];
    let end = body.find('}').unwrap_or(body.len());
    body[..end]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect()
}

/// WGSL cannot interpolate an integer, so an integer crossing between
/// stages has to say `@interpolate(flat)` explicitly. Naga's frontend
/// fills the default in silently - `apply_default_interpolation` maps
/// Sint/Uint to Flat - so the parsed module is identical either way and
/// only the source text can show the omission. WebKit does not fill it
/// in: it rejects the module, taking down every page that uses it.
#[test]
fn integer_varyings_are_flat() {
    let mut missing = Vec::new();
    for &(name, subs) in SHADERS {
        let (code, module) = parse(name, subs);
        for st in inter_stage_structs(&module) {
            for member in struct_members(&code, &st) {
                let is_integer = ["u32", "i32"]
                    .iter()
                    .any(|int| member.contains(&format!(": {}", int)) || member.contains(int));
                if member.contains("@location(")
                    && is_integer
                    && !member.contains("@interpolate(flat)")
                {
                    missing.push(format!("{} / {} / {}", name, st, member));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "integer values crossing stages must be @interpolate(flat):\n  {}",
        missing.join("\n  ")
    );
}

/// The line pipeline is laid out with only globals (0) and a colour
/// uniform (1). A group-2 requirement would need a dummy shape bind
/// group on every particle/insect draw.
#[test]
fn debug_line_shader_does_not_use_a_shape_bind_group() {
    let (code, _module) = parse("debug", &[]);
    assert!(
        code.contains("@group(1)"),
        "debug.wgsl should bind the colour uniform at group 1"
    );
    assert!(
        !code.contains("@group(2)"),
        "debug.wgsl binds group 2, but line draws only set groups 0 and 1"
    );
}
