use m3d::{CollisionQuad, DrawTriangle, Geometry, FullModel, Mesh, Model, Slot, Debrie};

use ron;

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

type RefModel = Model<Mesh<String>, Mesh<String>>;


pub fn export(full: FullModel, model_path: &PathBuf) {
    const BODY_PATH: &str = "body.obj";
    const SHAPE_PATH: &str = "body-shape.obj";

    let dir_path = model_path.parent().unwrap();

    let model = RefModel {
        body: full.body.map(|geom| {
            geom.save_obj(dir_path.join(BODY_PATH))
                .unwrap();
            BODY_PATH.to_string()
        }),
        shape: full.shape.map(|geom| {
            geom.save_obj(dir_path.join(SHAPE_PATH))
                .unwrap();
            SHAPE_PATH.to_string()
        }),
        dimensions: full.dimensions,
        max_radius: full.max_radius,
        color: full.color,
        wheels: full.wheels
            .into_iter()
            .enumerate()
            .map(|(i, wheel)| {
                wheel.map(|mesh| {
                    mesh.map(|geom| {
                        let name = format!("wheel{}.obj", i);
                        geom.save_obj(dir_path.join(&name)).unwrap();
                        name
                    })
                })
            })
            .collect(),
        debris: full.debris
            .into_iter()
            .enumerate()
            .map(|(i, debrie)| Debrie {
                mesh: debrie.mesh.map(|geom| {
                    let name = format!("debrie{}.obj", i);
                    geom.save_obj(dir_path.join(&name)).unwrap();
                    name
                }),
                shape: debrie.shape.map(|geom| {
                    let name = format!("debrie{}-shape.obj", i);
                    geom.save_obj(dir_path.join(&name)).unwrap();
                    name
                }),
            })
            .collect(),
        slots: Slot::map_all(full.slots, |mesh, i| {
            mesh.map(|geom| {
                let name = format!("slot{}.obj", i);
                geom.save_obj(dir_path.join(&name)).unwrap();
                name
            })
        }),
    };

    let string = ron::ser::to_string_pretty(&model, ron::ser::PrettyConfig::default()).unwrap();
    let mut model_file = File::create(model_path).unwrap();
    write!(model_file, "{}", string).unwrap();
}

pub fn import(model_path: &PathBuf) -> FullModel {
    let dir_path = model_path.parent().unwrap();
    let model_file = File::open(model_path).unwrap();
    let model = ron::de::from_reader::<_, RefModel>(model_file).unwrap();

    let resolve_geom_draw = |name| -> Geometry<DrawTriangle> { Geometry::load_obj(dir_path.join(name)) };
    let resolve_geom_coll = |name| -> Geometry<CollisionQuad> { Geometry::load_obj(dir_path.join(name)) };
    let resolve_mesh = |mesh: Mesh<String>| { mesh.map(&resolve_geom_draw) };

    FullModel {
        body: model.body.map(&resolve_geom_draw),
        shape: model.shape.map(&resolve_geom_coll),
        dimensions: model.dimensions,
        max_radius: model.max_radius,
        color: model.color,
        wheels: model.wheels
            .into_iter()
            .map(|wheel| wheel.map(&resolve_mesh))
            .collect(),
        debris: model.debris
            .into_iter()
            .map(|debrie| Debrie {
                mesh: debrie.mesh.map(&resolve_geom_draw),
                shape: debrie.shape.map(&resolve_geom_coll),
            })
            .collect(),
        slots: Slot::map_all(model.slots, |mesh, _| {
            resolve_mesh(mesh)
        }),
    }
}
