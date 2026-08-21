use strength_reduce::StrengthReducedU32;

use rayon::prelude::*;

use crate::color::{Color, RGB};

mod private {
    pub trait Sealed {}
}

pub trait Pixel: private::Sealed + Copy + Send + Sync {}

impl private::Sealed for Color {}
impl private::Sealed for RGB {}
impl Pixel for Color {}
impl Pixel for RGB {}

#[derive(Debug, Clone)]
pub struct Image<T: Pixel> {
    width: u32,
    height: u32,
    pixels: Vec<T>,
}

impl<T: Pixel> Image<T> {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn raw(&self) -> &[T] {
        &self.pixels
    }

    pub fn from_fn<F>(width: u32, height: u32, f: F) -> Image<T>
    where
        F: Fn(u32, u32) -> T + Send + Sync,
    {
        let w = StrengthReducedU32::new(width);

        let pixels = (0..width * height)
            .into_par_iter()
            .map(|i| {
                let (y, x) = StrengthReducedU32::div_rem(i, w);
                f(x, y)
            })
            .collect();

        Self {
            width,
            height,
            pixels,
        }
    }
}

impl Image<Color> {
    pub fn to_rgb(self) -> Image<RGB> {
        Image {
            width: self.width,
            height: self.height,
            pixels: self.pixels.into_iter().map(|x| x.to_rgb()).collect(),
        }
    }

    pub fn to_rgb_copy(&self) -> Image<RGB> {
        self.clone().to_rgb()
    }
}
