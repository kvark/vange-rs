pub const NORMALIZER: f32 = 124.0;
pub const NUM_COLOR_IDS: u32 = 25;

#[derive(Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ColorId {
    Reserved = 0,
    Body = 1,
    Window = 2,
    Wheel = 3,
    Defense = 4,
    Weapon = 5,
    Tube = 6,
    BodyRed = 7,
    BodyBlue = 8,
    BodyYellow = 9,
    BodyGray = 10,
    YellowCharged = 11,
    Custom0 = 12,
    Custom1 = 13,
    Custom2 = 14,
    Custom3 = 15,
    Custom4 = 16,
    Custom5 = 17,
    Custom6 = 18,
    Custom7 = 19,
    Black = 20,
    BodyGreen = 21,
    SkyFarmerKernboo = 22,
    SkyFarmerPipetka = 23,
    RottenItem = 24,
}

#[derive(Copy, Clone, Debug)]
pub struct Vertex {
    pub pos: u16,
    pub normal: u16,
}

impl Vertex {
    pub const DUMMY: Self = Vertex {
        pos: !0,
        normal: !0,
    };
}

pub struct DrawTriangle {
    pub vertices: [Vertex; 3],
    pub flat_normal: [i8; 3],
    pub material: [u32; 2],
}

pub struct CollisionQuad {
    pub vertices: [u16; 4],
    pub middle: [i8; 3],
    pub flat_normal: [i8; 3],
}

#[derive(Default)]
pub struct Geometry<P> {
    pub positions: Vec<[i8; 3]>,
    pub normals: Vec<[i8; 3]>,
    pub polygons: Vec<P>,
}
