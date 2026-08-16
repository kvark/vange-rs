//! Reader for the `*.vlc` tables that sit next to a level's `*.vot` files.
//!
//! `VLload` of `src/units/moveland.cpp` reads four of them out of the world's
//! `data.vot` folder, each a signature, an entry count and a flat array:
//!
//! | file            | signature | contents                          |
//! |-----------------|-----------|-----------------------------------|
//! | `tnttable.vlc`  | `VLT1`    | explosive barrels                 |
//! | `mlctable.vlc`  | `VLM1`    | moving-land clone markers         |
//! | `snstable.vlc`  | `VLS1`    | sensors                           |
//! | `dngtable.vlc`  | `VLD1`    | danger zones                      |
//!
//! Only the two that the moving land cares about are read here; barrels and
//! danger zones belong to systems this port does not have yet.

use byteorder::{LittleEndian as E, ReadBytesExt};

use std::{
    fs::File,
    io::{self, BufReader, Read},
    path::Path,
};

/// `SensorTypeList` of the original. Only the variants the moving land reacts
/// to are named; everything else keeps its raw number.
pub mod sensor_kind {
    pub const NONE: i32 = 0;
    /// The plain proximity sensor that drives doors and bridges.
    pub const SENSOR: i32 = 1;
    pub const IMPULSE: i32 = 2;
    pub const SPOT: i32 = 3;
    pub const ESCAVE: i32 = 4;
    pub const PASSAGE: i32 = 5;
    pub const TRAIN: i32 = 6;
    pub const TRAP: i32 = 7;
}

/// One entry of `snstable.vlc` - a named trigger volume.
#[derive(Debug)]
pub struct Sensor {
    pub pos: (i32, i32, i32),
    /// One of [`sensor_kind`].
    pub kind: i32,
    pub radius: i32,
    /// How `location.lst` refers to this sensor. May be empty.
    pub name: String,
    /// `z0`/`z1` - the altitude band the sensor reacts in.
    pub z_range: (i32, i32),
    /// `vData` - direction of the push, for impulse sensors.
    pub direction: (i32, i32, i32),
    pub power: i32,
    pub data5: i32,
    pub data6: i32,
}

/// One entry of `mlctable.vlc` - "put a copy of moving land `source` here".
#[derive(Debug)]
pub struct CloneMarker {
    pub pos: (i32, i32, i32),
    /// Index into the level's moving-land table.
    pub source: i32,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    BadSignature([u8; 4]),
    /// The entry count is negative or absurd for the file's size.
    BadCount(i32),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Error::Io(ref e) => write!(f, "I/O error: {}", e),
            Error::BadSignature(sig) => {
                write!(
                    f,
                    "unexpected VLC signature {:?}",
                    String::from_utf8_lossy(&sig)
                )
            }
            Error::BadCount(n) => write!(f, "implausible entry count {}", n),
        }
    }
}

impl std::error::Error for Error {}

/// Reads the 4-byte signature and the entry count shared by every table.
fn open_table<I: Read>(input: &mut I, expected: &[u8; 4]) -> Result<i32, Error> {
    let mut signature = [0u8; 4];
    input.read_exact(&mut signature)?;
    if &signature != expected {
        return Err(Error::BadSignature(signature));
    }
    let count = input.read_i32::<E>()?;
    if count < 0 {
        return Err(Error::BadCount(count));
    }
    Ok(count)
}

/// Loads `snstable.vlc` from a level's `data.vot` folder. A missing file just
/// means the level has no sensors.
pub fn load_sensors(dir: &Path) -> Vec<Sensor> {
    load_table(dir, "snstable.vlc", b"VLS1", read_sensor)
}

/// Loads `mlctable.vlc`. Note that the original game reads this file's header
/// and then skips the entries - only the map editor ever looks at them - so
/// nothing instantiates these markers automatically.
pub fn load_clone_markers(dir: &Path) -> Vec<CloneMarker> {
    load_table(dir, "mlctable.vlc", b"VLM1", read_clone_marker)
}

fn load_table<T, F>(dir: &Path, name: &str, signature: &[u8; 4], read: F) -> Vec<T>
where
    F: Fn(&mut BufReader<File>) -> Result<T, Error>,
{
    let path = dir.join(name);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log::warn!("Unable to open {:?}: {}", path, e);
            }
            return Vec::new();
        }
    };

    let mut input = BufReader::new(file);
    let count = match open_table(&mut input, signature) {
        Ok(count) => count,
        Err(e) => {
            log::error!("Unable to read {:?}: {}", path, e);
            return Vec::new();
        }
    };

    let mut entries = Vec::with_capacity(count as usize);
    for index in 0..count {
        match read(&mut input) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                log::error!("Unable to read entry {} of {:?}: {}", index, path, e);
                break;
            }
        }
    }
    log::info!("Loaded {} entries from {:?}", entries.len(), path);
    entries
}

fn read_sensor<I: Read>(input: &mut I) -> Result<Sensor, Error> {
    let pos = read_vector(input)?;
    let kind = input.read_i32::<E>()?;
    let radius = input.read_i32::<E>()?;

    let name_len = input.read_i32::<E>()?;
    if !(0..=0x1000).contains(&name_len) {
        return Err(Error::BadCount(name_len));
    }
    let mut raw_name = vec![0u8; name_len as usize];
    input.read_exact(&mut raw_name)?;
    let name = String::from_utf8_lossy(&raw_name).into_owned();

    let z0 = input.read_i32::<E>()?;
    let direction = read_vector(input)?;
    let power = input.read_i32::<E>()?;
    let z1 = input.read_i32::<E>()?;
    let data5 = input.read_i32::<E>()?;
    let data6 = input.read_i32::<E>()?;

    Ok(Sensor {
        pos,
        kind,
        radius,
        name,
        z_range: (z0, z1),
        direction,
        power,
        data5,
        data6,
    })
}

fn read_clone_marker<I: Read>(input: &mut I) -> Result<CloneMarker, Error> {
    Ok(CloneMarker {
        pos: read_vector(input)?,
        source: input.read_i32::<E>()?,
    })
}

fn read_vector<I: Read>(input: &mut I) -> Result<(i32, i32, i32), Error> {
    Ok((
        input.read_i32::<E>()?,
        input.read_i32::<E>()?,
        input.read_i32::<E>()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;

    fn push_i32(data: &mut Vec<u8>, value: i32) {
        data.write_i32::<E>(value).unwrap();
    }

    fn sensor_table() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"VLS1");
        push_i32(&mut data, 2);
        for (name, kind) in [("bridge_west", sensor_kind::SENSOR), ("", 9)] {
            for v in [10, 20, 30] {
                push_i32(&mut data, v);
            }
            push_i32(&mut data, kind);
            push_i32(&mut data, 7); // radius
            push_i32(&mut data, name.len() as i32);
            data.extend_from_slice(name.as_bytes());
            push_i32(&mut data, 23); // z0
            for v in [1, 2, 3] {
                push_i32(&mut data, v); // vData
            }
            push_i32(&mut data, 50); // Power
            push_i32(&mut data, 37); // z1
            push_i32(&mut data, 5);
            push_i32(&mut data, 6);
        }
        data
    }

    #[test]
    fn sensor_layout() {
        let data = sensor_table();
        let mut input = data.as_slice();
        assert_eq!(open_table(&mut input, b"VLS1").unwrap(), 2);

        let first = read_sensor(&mut input).unwrap();
        assert_eq!(first.pos, (10, 20, 30));
        assert_eq!(first.kind, sensor_kind::SENSOR);
        assert_eq!(first.radius, 7);
        assert_eq!(first.name, "bridge_west");
        assert_eq!(first.z_range, (23, 37));
        assert_eq!(first.direction, (1, 2, 3));
        assert_eq!(first.power, 50);
        assert_eq!((first.data5, first.data6), (5, 6));

        // A zero-length name is legal and reads as empty.
        let second = read_sensor(&mut input).unwrap();
        assert_eq!(second.name, "");
        assert_eq!(second.kind, 9);
    }

    #[test]
    fn clone_marker_layout() {
        let mut data = Vec::new();
        data.extend_from_slice(b"VLM1");
        push_i32(&mut data, 1);
        for v in [100, 200, 300, 4] {
            push_i32(&mut data, v);
        }

        let mut input = data.as_slice();
        assert_eq!(open_table(&mut input, b"VLM1").unwrap(), 1);
        let marker = read_clone_marker(&mut input).unwrap();
        assert_eq!(marker.pos, (100, 200, 300));
        assert_eq!(marker.source, 4);
    }

    #[test]
    fn wrong_signature_is_rejected() {
        let mut data = sensor_table();
        data[1] = b'X';
        let mut input = data.as_slice();
        assert!(matches!(
            open_table(&mut input, b"VLS1"),
            Err(Error::BadSignature(_))
        ));
    }

    #[test]
    fn absurd_name_length_is_rejected() {
        let mut data = Vec::new();
        for v in [0, 0, 0, sensor_kind::SENSOR, 1, i32::MAX] {
            push_i32(&mut data, v);
        }
        let mut input = data.as_slice();
        assert!(matches!(
            read_sensor(&mut input),
            Err(Error::BadCount(i32::MAX))
        ));
    }

    #[test]
    fn missing_files_yield_nothing() {
        let dir = Path::new("/definitely/not/here");
        assert!(load_sensors(dir).is_empty());
        assert!(load_clone_markers(dir).is_empty());
    }

    #[test]
    fn load_sensors_reads_the_file() {
        let dir = std::env::temp_dir().join(format!("vange-rs-vlc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("snstable.vlc"), sensor_table()).unwrap();

        let sensors = load_sensors(&dir);
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(sensors.len(), 2);
        assert_eq!(sensors[0].name, "bridge_west");
    }
}
