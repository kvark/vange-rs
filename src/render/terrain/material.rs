//! Load-time fit of each terrain type's `BEGCOLOR..=ENDCOLOR` ramp to a
//! two-tone linear-RGB material (shadow + lit). The shaders lerp these
//! in sRGB, decode, and light with a real colored sun and local lights
//! — no runtime `t_Palette` index.

use crate::level::TerrainConfig;

/// Least-squares line through a ramp in sRGB byte space, evaluated at
/// t=0 (shadow) and t=1 (fully lit). The first palette entry of each
/// ramp is halved by `PalettePrepare`, so a raw endpoint pair undershoots
/// the mid-ramp olive/ochre the 1998 look actually spends most of its
/// time in; the fit recovers that mid.
///
/// `colors.end` is inclusive, matching `TerrainConfig`.
pub fn fit_ramp(palette: &[[u8; 4]], colors: std::ops::Range<u8>) -> ([u8; 3], [u8; 3]) {
    let start = colors.start as usize;
    let end = colors.end as usize;
    if start >= palette.len() {
        return ([0; 3], [0; 3]);
    }
    let end = end.min(palette.len() - 1).max(start);
    let n = end - start + 1;
    if n == 1 {
        let c = palette[start];
        let rgb = [c[0], c[1], c[2]];
        return (rgb, rgb);
    }

    let mut shadow = [0u8; 3];
    let mut lit = [0u8; 3];
    let nf = n as f32;
    let mut sum_t = 0.0f32;
    let mut sum_t2 = 0.0f32;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        sum_t += t;
        sum_t2 += t * t;
    }
    let det = nf * sum_t2 - sum_t * sum_t;
    for k in 0..3 {
        let mut sum_c = 0.0f32;
        let mut sum_tc = 0.0f32;
        for i in 0..n {
            let t = i as f32 / (n - 1) as f32;
            let c = f32::from(palette[start + i][k]);
            sum_c += c;
            sum_tc += t * c;
        }
        let slope = (nf * sum_tc - sum_t * sum_c) / det;
        let intercept = (sum_c - slope * sum_t) / nf;
        shadow[k] = intercept.round().clamp(0.0, 255.0) as u8;
        lit[k] = (intercept + slope).round().clamp(0.0, 255.0) as u8;
    }
    (shadow, lit)
}

/// Two-row texture texels, row 0 = shadow sRGB, row 1 = lit sRGB,
/// packed as Rgba8Unorm (the shader decodes after the lerp).
pub fn pack_material_texels(palette: &[[u8; 4]], terrains: &[TerrainConfig]) -> Vec<[u8; 4]> {
    let n = terrains.len();
    let mut data = vec![[0u8, 0, 0, 255]; n.saturating_mul(2)];
    for (i, terr) in terrains.iter().enumerate() {
        let (shadow, lit) = fit_ramp(palette, terr.colors.clone());
        data[i] = [shadow[0], shadow[1], shadow[2], 255];
        data[n + i] = [lit[0], lit[1], lit[2], 255];
    }
    data
}

#[cfg(test)]
mod tests {
    use super::{fit_ramp, pack_material_texels};
    use crate::level::TerrainConfig;

    fn srgb_u8_to_linear(c: u8) -> f32 {
        let x = f32::from(c) / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    }

    #[test]
    fn srgb_zero_and_white() {
        assert!((srgb_u8_to_linear(0) - 0.0).abs() < 1e-6);
        assert!((srgb_u8_to_linear(255) - 1.0).abs() < 1e-5);
        // 8/255 is still in the linear toe (threshold is 0.04045).
        assert!((srgb_u8_to_linear(8) - 8.0 / 255.0 / 12.92).abs() < 1e-6);
    }

    #[test]
    fn a_linear_srgb_ramp_recovers_its_endpoints() {
        let mut pal = [[0u8; 4]; 256];
        for i in 0..32 {
            let v = (8 + i * 4) as u8;
            pal[32 + i] = [v, v.saturating_add(8), v / 2, 255];
        }
        let (shadow, lit) = fit_ramp(&pal, 32..63);
        assert_eq!(shadow, [8, 16, 4]);
        assert_eq!(lit, [8 + 31 * 4, 16 + 31 * 4, 4 + 31 * 2]);
    }

    #[test]
    fn a_halved_first_entry_does_not_pull_the_mid_down() {
        // Fostral typ 1 shape: first entry halved, then a rising olive ramp.
        let mut pal = [[0u8; 4]; 256];
        for i in 0..32 {
            let t = i as f32 / 31.0;
            pal[32 + i] = [
                (24.0 + t * 128.0).round() as u8,
                (32.0 + t * 136.0).round() as u8,
                (16.0 + t * 112.0).round() as u8,
                255,
            ];
        }
        pal[32] = [8, 12, 4, 255]; // PalettePrepare halving
        let (shadow, lit) = fit_ramp(&pal, 32..63);
        // Mid of the fitted line should sit near the real mid, not the
        // darkened-first-entry lerp (which lands ~10 bytes too dark).
        let mid = [
            (u16::from(shadow[0]) + u16::from(lit[0]) + 1) / 2,
            (u16::from(shadow[1]) + u16::from(lit[1]) + 1) / 2,
            (u16::from(shadow[2]) + u16::from(lit[2]) + 1) / 2,
        ];
        let actual_mid = pal[32 + 16];
        for k in 0..3 {
            let err = (mid[k] as i16 - i16::from(actual_mid[k])).unsigned_abs();
            assert!(
                err <= 6,
                "channel {k}: fitted mid {} vs actual {}",
                mid[k],
                actual_mid[k]
            );
        }
        assert!(shadow[0] > 8, "fit should lift the halved first entry");
    }

    #[test]
    fn pack_writes_shadow_then_lit_rows() {
        let mut pal = [[0u8; 4]; 256];
        pal[0] = [10, 20, 30, 255];
        pal[1] = [40, 50, 60, 255];
        pal[2] = [70, 80, 90, 255];
        pal[3] = [100, 110, 120, 255];
        let terrains = [
            TerrainConfig {
                colors: 0..1,
                ..TerrainConfig::default()
            },
            TerrainConfig {
                colors: 2..3,
                ..TerrainConfig::default()
            },
        ];
        let texels = pack_material_texels(&pal, &terrains);
        assert_eq!(texels.len(), 4);
        // Row 0: both shadows. Linear 2-entry ramps recover the endpoints.
        assert_eq!(texels[0][..3], [10, 20, 30]);
        assert_eq!(texels[1][..3], [70, 80, 90]);
        // Row 1: both lits.
        assert_eq!(texels[2][..3], [40, 50, 60]);
        assert_eq!(texels[3][..3], [100, 110, 120]);
    }
}
