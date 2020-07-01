use vangers::level::{
    Level, LevelData, DELTA_MASK, DELTA_SHIFT0, DELTA_SHIFT1, DOUBLE_LEVEL, NUM_TERRAINS,
    TERRAIN_SHIFT,
};

fn avg(a: u8, b: u8) -> u8 {
    (a >> 1) + (b >> 1) + (a & b & 1)
}

pub fn extract_palette(level: &Level) -> Vec<u8> {
    level
        .terrains
        .iter()
        .flat_map(|terr| level.palette[terr.colors.end as usize - 1].iter().cloned())
        .collect()
}

pub struct LevelLayers {
    pub size: (u32, u32),
    pub het0: Vec<u8>,
    pub het1: Vec<u8>,
    pub delta: Vec<u8>,
    pub mat0: Vec<u8>,
    pub mat1: Vec<u8>,
}

impl LevelLayers {
    pub fn new(size: (u32, u32)) -> Self {
        let total = (size.0 * size.1) as usize;
        LevelLayers {
            size,
            het0: Vec::with_capacity(total),
            het1: Vec::with_capacity(total),
            delta: Vec::with_capacity(total),
            mat0: Vec::with_capacity(total / 2),
            mat1: Vec::with_capacity(total / 2),
        }
    }

    pub fn from_level_data(data: &LevelData) -> Self {
        let mut ll = LevelLayers::new((data.size.0 as u32, data.size.1 as u32));
        ll.import(data);
        ll
    }

    fn import(&mut self, data: &LevelData) {
        for y in 0..data.size.1 as usize {
            let range = y * data.size.0 as usize..(y + 1) * data.size.0 as usize;
            let hrow = &data.height[range.clone()];
            let mrow = &data.meta[range];
            for ((&h0, &h1), (&m0, &m1)) in hrow
                .iter()
                .step_by(2)
                .zip(hrow[1..].iter().step_by(2))
                .zip(mrow.iter().step_by(2).zip(mrow[1..].iter().step_by(2)))
            {
                let t0 = (m0 >> TERRAIN_SHIFT) & (NUM_TERRAINS as u8 - 1);
                let t1 = (m1 >> TERRAIN_SHIFT) & (NUM_TERRAINS as u8 - 1);
                if m0 & DOUBLE_LEVEL != 0 {
                    let d =
                        ((m0 & DELTA_MASK) << DELTA_SHIFT0) + ((m1 & DELTA_MASK) << DELTA_SHIFT1);
                    //assert!(h0 + d <= h1); //TODO: figure out why this isn't true
                    self.het0.push(h0);
                    self.het0.push(h0);
                    self.het1.push(h1);
                    self.het1.push(h1);
                    self.delta.push(d);
                    self.delta.push(d);
                    self.mat0.push(t0 | (t0 << 4));
                    self.mat1.push(t1 | (t1 << 4));
                } else {
                    self.het0.push(h0);
                    self.het0.push(h1);
                    self.het1.push(h0);
                    self.het1.push(h1);
                    self.delta.push(0);
                    self.delta.push(0);
                    self.mat0.push(t0 | (t1 << 4));
                    self.mat1.push(t0 | (t1 << 4));
                }
            }
        }
    }

    pub fn export(self) -> LevelData {
        let total = self.size.0 as usize * self.size.1 as usize;
        let mut height = Vec::with_capacity(total);
        let mut meta = Vec::with_capacity(total);

        for (((&h0a, &h0b), (&h1a, &h1b)), ((&da, &db), (&mat0, &mat1))) in self
            .het0
            .iter()
            .step_by(2)
            .zip(self.het0[1..].iter().step_by(2))
            .zip(
                self.het1
                    .iter()
                    .step_by(2)
                    .zip(self.het1[1..].iter().step_by(2)),
            )
            .zip(
                self.delta
                    .iter()
                    .step_by(2)
                    .zip(self.delta[1..].iter().step_by(2))
                    .zip(self.mat0.iter().zip(&self.mat1)),
            )
        {
            //assert!(h0a + da <= h1a && h0b + db <= h1b);
            let delta = avg(da, db);
            if delta != 0 {
                //Note: mat0b and mat1a are ignored here, assuming the same as mat0a and mat1b respectively
                meta.push(DOUBLE_LEVEL | ((mat0 & 0xF) << TERRAIN_SHIFT) | (delta >> 2));
                meta.push(DOUBLE_LEVEL | ((mat1 >> 4) << TERRAIN_SHIFT) | (delta & DELTA_MASK));
                height.push(avg(h0a, h0b));
                height.push(avg(h1a, h1b));
            } else {
                //Note: mat1 and deltas are ignored here, assuming mat0 == mat1
                height.push(avg(h0a, h1a));
                height.push(avg(h0b, h1b));
                meta.push((mat0 & 0xF) << TERRAIN_SHIFT);
                meta.push((mat0 >> 4) << TERRAIN_SHIFT);
            }
        }

        LevelData {
            size: (self.size.0 as i32, self.size.1 as i32),
            meta,
            height,
        }
    }
}
