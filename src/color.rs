use super::vec3::Vec3;

pub type Color = Vec3<f32>;
pub type RGB   = Vec3<u8>;

impl Color {
    pub fn to_rgb(self) -> RGB {
        let linear_to_srgb = |c: f32| {if c <= 0.003_130_8
                                             { c * 12.92 }
                                       else  { 1.055 * c.powf(1.0 / 2.4) - 0.055 }};
        let to_u8 = |c: f32| (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;

        self.map(to_u8)
    }
}
