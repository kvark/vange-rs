use std::io::{Read};
use byteorder::{LittleEndian as E, ReadBytesExt};
use gfx;
use gfx::format::I8Norm;


const SCALE: f32 = 1.0 / 256.0;

gfx_vertex_struct!( Vertex {
    pos: [f32; 4] = "a_Pos",
    color: [u32; 2] = "a_Color",
    normal: [I8Norm; 4] = "a_Normal",
});

pub fn load_c3d<I, R, F>(source: &mut I, factory: &mut F)
                -> (gfx::handle::Buffer<R, Vertex>, gfx::Slice<R>) where
    I: ReadBytesExt,
    R: gfx::Resources,
    F: gfx::traits::FactoryExt<R>,
{
    let version = source.read_u32::<E>().unwrap();
    assert_eq!(version, 8);
    let num_positions = source.read_u32::<E>().unwrap();
    let num_normals   = source.read_u32::<E>().unwrap();
    let num_polygons  = source.read_u32::<E>().unwrap();
    let _total_verts  = source.read_u32::<E>().unwrap();

    let coord_max  = [
        source.read_u32::<E>().unwrap() as f32 * SCALE,
        source.read_u32::<E>().unwrap() as f32 * SCALE,
        source.read_u32::<E>().unwrap() as f32 * SCALE,
    ];
    let coord_min  = [
        source.read_u32::<E>().unwrap() as f32 * SCALE,
        source.read_u32::<E>().unwrap() as f32 * SCALE,
        source.read_u32::<E>().unwrap() as f32 * SCALE,
    ];
    let _parent_off = [
        source.read_u32::<E>().unwrap() as f32 * SCALE,
        source.read_u32::<E>().unwrap() as f32 * SCALE,
        source.read_u32::<E>().unwrap() as f32 * SCALE,
    ];
    let _max_radius = source.read_u32::<E>().unwrap();
    let _parent_rot = [source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap()];
    let _extras    = [source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap()];

    info!("\tReading {} positions...", num_positions);
    let mut positions = Vec::with_capacity(num_positions as usize);
    for _ in 0 .. num_positions {
        for _ in 0..3 {
            source.read_u32::<E>().unwrap(); //unknown
        }
        let mut pos = [0u8; 3];
        source.read(&mut pos).unwrap();
        let _sort_info = source.read_u32::<E>().unwrap();
        positions.push([
            coord_min[0] + (pos[0] as f32 / 255.0) * (coord_max[0] - coord_min[0]),
            coord_min[1] + (pos[1] as f32 / 255.0) * (coord_max[1] - coord_min[1]),
            coord_min[2] + (pos[2] as f32 / 255.0) * (coord_max[2] - coord_min[2]),
            1.0,
        ]);
    }

    info!("\tReading {} normals...", num_normals);
    let mut normals = Vec::with_capacity(num_normals as usize);
    for _ in 0 .. num_normals {
        let mut norm = [0u8; 4];
        source.read(&mut norm).unwrap();
        let _sort_info = source.read_u32::<E>().unwrap();
        normals.push([
            I8Norm(norm[0] as i8), I8Norm(norm[1] as i8), I8Norm(norm[2] as i8), I8Norm(0),
        ]);
    }

    info!("\tReading {} polygons...", num_polygons);
    let mut vertices = Vec::with_capacity(num_polygons as usize * 3);
    for _ in 0 .. num_polygons {
        let num_corners = source.read_u32::<E>().unwrap();
        assert_eq!(num_corners, 3);
        let _sort_info = source.read_u32::<E>().unwrap();
        let color = [source.read_u32::<E>().unwrap(), source.read_u32::<E>().unwrap()];
        let mut dummy = [0, 8];
        source.read(&mut dummy[..8]).unwrap(); //skip flat normal
        source.read(&mut dummy[..3]).unwrap(); //skip middle point
        for _ in 0..3 {
            let pid = source.read_u32::<E>().unwrap();
            let nid = source.read_u32::<E>().unwrap();
            vertices.push(Vertex {
                pos: positions[pid as usize],
                color: color,
                normal: normals[nid as usize],
            });
        }
    }

    //vertices.sort();
    //vertices.dedup();
    info!("\tDerived {} unique vertices...", vertices.len());

    factory.create_vertex_buffer_with_slice(&vertices, ())
}
