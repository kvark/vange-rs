use m3d::{
    AnimatedMesh, CollisionQuad, ColorId, Debrie, DrawTriangle, FullModel, Geometry, Mesh, Model,
    Polygon, Slot, Vertex, NORMALIZER, NUM_COLOR_IDS,
};

use obj::{IndexTuple, Obj};

use std::path::Path;
use std::{
    fs,
    io::{Result as IoResult, Write as _},
    path::PathBuf,
};

type RefModel = Model<Mesh<String>, Mesh<String>>;
type RefAnimatedMesh = AnimatedMesh<String>;
type DrawAnimatedMesh = AnimatedMesh<Geometry<DrawTriangle>>;

const MAT_NAME: &'static str = "object.mtl";

pub fn export_m3d(full: FullModel, model_path: &Path) {
    const BODY_PATH: &str = "body.obj";
    const SHAPE_PATH: &str = "body-shape.obj";

    let dir_path = model_path.parent().unwrap();

    let model = RefModel {
        body: full.body.map(|geom| {
            save_draw_geometry(&geom, dir_path.join(BODY_PATH)).unwrap();
            BODY_PATH.to_string()
        }),
        shape: full.shape.map(|geom| {
            save_collision_geometry(&geom, dir_path.join(SHAPE_PATH)).unwrap();
            SHAPE_PATH.to_string()
        }),
        bound: full.bound,
        color: full.color,
        wheels: full
            .wheels
            .into_iter()
            .enumerate()
            .map(|(i, wheel)| {
                wheel.map(|mesh| {
                    mesh.map(|geom| {
                        let name = format!("wheel{}.obj", i);
                        save_draw_geometry(&geom, dir_path.join(&name)).unwrap();
                        name
                    })
                })
            })
            .collect(),
        debris: full
            .debris
            .into_iter()
            .enumerate()
            .map(|(i, debrie)| Debrie {
                mesh: debrie.mesh.map(|geom| {
                    let name = format!("debrie{}.obj", i);
                    save_draw_geometry(&geom, dir_path.join(&name)).unwrap();
                    name
                }),
                shape: debrie.shape.map(|geom| {
                    let name = format!("debrie{}-shape.obj", i);
                    save_collision_geometry(&geom, dir_path.join(&name)).unwrap();
                    name
                }),
            })
            .collect(),
        slots: Slot::map_all(full.slots, |mesh, i| {
            mesh.map(|geom| {
                let name = format!("slot{}.obj", i);
                save_draw_geometry(&geom, dir_path.join(&name)).unwrap();
                name
            })
        }),
    };

    let string = ron::ser::to_string_pretty(&model, ron::ser::PrettyConfig::default()).unwrap();
    fs::write(model_path, string).unwrap();
}

pub fn import_m3d(model_path: &Path) -> FullModel {
    let dir_path = model_path.parent().unwrap();
    let model_file = fs::File::open(model_path).unwrap();
    let model = ron::de::from_reader::<_, RefModel>(model_file).unwrap();

    let resolve_geom_draw = |name| -> Geometry<DrawTriangle> { load_geometry(dir_path.join(name)) };
    let resolve_geom_coll =
        |name| -> Geometry<CollisionQuad> { load_geometry(dir_path.join(name)) };
    let resolve_mesh = |mesh: Mesh<String>| mesh.map(&resolve_geom_draw);

    FullModel {
        body: model.body.map(&resolve_geom_draw),
        shape: model.shape.map(&resolve_geom_coll),
        bound: model.bound,
        color: model.color,
        wheels: model
            .wheels
            .into_iter()
            .map(|wheel| wheel.map(&resolve_mesh))
            .collect(),
        debris: model
            .debris
            .into_iter()
            .map(|debrie| Debrie {
                mesh: debrie.mesh.map(&resolve_geom_draw),
                shape: debrie.shape.map(&resolve_geom_coll),
            })
            .collect(),
        slots: Slot::map_all(model.slots, |mesh, _| resolve_mesh(mesh)),
    }
}

pub fn export_a3d(a3d: DrawAnimatedMesh, mesh_path: &Path) {
    let dir_path = mesh_path.parent().unwrap();

    let amesh = RefAnimatedMesh {
        bound: a3d.bound,
        color: a3d.color,
        meshes: a3d
            .meshes
            .into_iter()
            .enumerate()
            .map(|(i, mesh)| {
                let name = format!("body-{}.obj", i + 1);
                mesh.map(|geom| {
                    save_draw_geometry(&geom, dir_path.join(&name)).unwrap();
                    name
                })
            })
            .collect(),
    };

    let string = ron::ser::to_string_pretty(&amesh, ron::ser::PrettyConfig::default()).unwrap();
    fs::write(mesh_path, string).unwrap();
}

pub fn import_a3d(mesh_path: &Path) -> DrawAnimatedMesh {
    let dir_path = mesh_path.parent().unwrap();
    let amesh_file = fs::File::open(mesh_path).unwrap();
    let a3d = ron::de::from_reader::<_, RefAnimatedMesh>(amesh_file).unwrap();
    DrawAnimatedMesh {
        bound: a3d.bound,
        color: a3d.color,
        meshes: a3d
            .meshes
            .into_iter()
            .map(|mesh| mesh.map(|name| load_geometry(dir_path.join(&name))))
            .collect(),
    }
}

fn map_color_id(id: u32) -> ColorId {
    use std::mem;
    if id < NUM_COLOR_IDS {
        unsafe { mem::transmute(id) }
    } else {
        println!("Unknown ColorId {:?}", id);
        ColorId::Reserved
    }
}

fn flatten_normal(poly: &[IndexTuple], normals: &[[f32; 3]]) -> [i8; 3] {
    let n = poly.iter().fold([0f32; 3], |u, IndexTuple(_, _, ni)| {
        let n = match ni {
            Some(index) => normals[*index],
            None => [0.0, 0.0, 0.0],
        };
        [u[0] + n[0], u[1] + n[1], u[2] + n[2]]
    });
    let m2 = n.iter().fold(0f32, |u, v| u + v * v);
    let scale = if m2 == 0.0 {
        0.0
    } else {
        NORMALIZER / m2.sqrt()
    };
    [
        (n[0] * scale) as i8,
        (n[1] * scale) as i8,
        (n[2] * scale) as i8,
    ]
}

fn flatten_pos(poly: &[IndexTuple], positions: &[[f32; 3]]) -> [i8; 3] {
    let m = poly.iter().fold([0f32; 3], |u, IndexTuple(pi, _, _)| {
        let p = positions[*pi];
        [u[0] + p[0], u[1] + p[1], u[2] + p[2]]
    });
    [
        (m[0] * 0.25) as i8,
        (m[1] * 0.25) as i8,
        (m[2] * 0.25) as i8,
    ]
}

pub fn save_palette(path: PathBuf, palette: &[u8]) -> IoResult<()> {
    if path.file_name().unwrap() != MAT_NAME {
        log::warn!("Saved material is different from the expected {}", MAT_NAME);
    }
    let mut dest = fs::File::create(&path)?;
    for (color_id, color) in palette.chunks(3).enumerate() {
        writeln!(dest, "newmtl {:?}", map_color_id(color_id as u32))?;
        writeln!(
            dest,
            "\tKd {} {} {}",
            color[0] as f32 / 255.0,
            color[1] as f32 / 255.0,
            color[2] as f32 / 255.0,
        )?;
    }
    Ok(())
}

pub fn save_draw_geometry(geom: &Geometry<DrawTriangle>, path: PathBuf) -> IoResult<()> {
    let mut dest = fs::File::create(&path)?;
    for p in geom.positions.iter() {
        writeln!(dest, "v {} {} {}", p[0], p[1], p[2])?;
    }
    writeln!(dest)?;
    for n in geom.normals.iter() {
        writeln!(
            dest,
            "vn {} {} {}",
            n[0] as f32 / NORMALIZER,
            n[1] as f32 / NORMALIZER,
            n[2] as f32 / NORMALIZER
        )?;
    }
    writeln!(dest)?;

    writeln!(dest, "mtllib {}", MAT_NAME)?;
    let mut mask = 0u32;
    for p in &geom.polygons {
        mask |= 1 << p.material[0];
    }
    for color_id in 0..NUM_COLOR_IDS {
        if mask & (1 << color_id) == 0 {
            continue;
        }
        writeln!(dest, "usemtl {:?}", map_color_id(color_id))?;
        for p in &geom.polygons {
            if p.material[0] != color_id {
                continue;
            }
            write!(dest, "f")?;
            for v in &p.vertices {
                write!(dest, " {}//{}", v.pos + 1, v.normal + 1)?;
            }
            writeln!(dest)?;
        }
    }

    Ok(())
}

pub fn save_collision_geometry(geom: &Geometry<CollisionQuad>, path: PathBuf) -> IoResult<()> {
    let mut dest = fs::File::create(&path).unwrap();
    for p in geom.positions.iter() {
        writeln!(dest, "v {} {} {}", p[0], p[1], p[2])?;
    }
    writeln!(dest)?;

    // replace the normals with flat normals
    for p in geom.polygons.iter() {
        writeln!(
            dest,
            "vn {} {} {}",
            p.flat_normal[0] as f32 / NORMALIZER,
            p.flat_normal[1] as f32 / NORMALIZER,
            p.flat_normal[2] as f32 / NORMALIZER
        )?;
    }
    writeln!(dest)?;

    for (i, p) in geom.polygons.iter().enumerate() {
        write!(dest, "f")?;
        for &pi in &p.vertices {
            write!(dest, " {}//{}", pi + 1, i + 1)?;
        }
        writeln!(dest)?;
    }

    Ok(())
}

pub fn load_geometry<P: Polygon>(path: PathBuf) -> Geometry<P> {
    let obj = Obj::load(&path).unwrap();

    let positions = obj
        .data
        .position
        .iter()
        .map(|p| {
            [
                p[0].min(NORMALIZER).max(-NORMALIZER) as i8,
                p[1].min(NORMALIZER).max(-NORMALIZER) as i8,
                p[2].min(NORMALIZER).max(-NORMALIZER) as i8,
            ]
        })
        .collect();
    let normals = obj
        .data
        .normal
        .iter()
        .map(|n| {
            [
                (n[0] * NORMALIZER) as i8,
                (n[1] * NORMALIZER) as i8,
                (n[2] * NORMALIZER) as i8,
            ]
        })
        .collect();

    let color_names = (0..NUM_COLOR_IDS)
        .map(|id| format!("{:?}", map_color_id(id)))
        .collect::<Vec<_>>();

    let data_ref = &obj.data;
    let polygons = obj
        .data
        .objects
        .iter()
        .flat_map(|object| {
            object.groups.iter().flat_map(|group| {
                let mut vertices = Vec::with_capacity(4);
                let color_id = color_names
                    .iter()
                    .position(|c| c == &group.name)
                    .unwrap_or(0);
                group.polys.iter().map(move |poly| {
                    vertices.clear();
                    for &IndexTuple(pi, _, ni) in poly.0.iter() {
                        vertices.push(Vertex {
                            pos: pi as u16,
                            normal: ni.unwrap_or(0) as u16,
                        })
                    }
                    P::new(
                        flatten_pos(&poly.0, &data_ref.position),
                        flatten_normal(&poly.0, &data_ref.normal),
                        [color_id as u32, 0],
                        &vertices,
                    )
                })
            })
        })
        .collect();

    Geometry {
        positions,
        normals,
        polygons,
    }
}
