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

    pub fn r(self) -> f32 {self.0}
    pub fn g(self) -> f32 {self.1}
    pub fn b(self) -> f32 {self.2}
}

impl RGB {
    pub fn r(self) -> u8 {self.0}
    pub fn g(self) -> u8 {self.1}
    pub fn b(self) -> u8 {self.2}
}
