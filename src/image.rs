use strength_reduce::StrengthReducedUsize;

use crate::color::{Color, RGB};

mod private {
    pub trait Sealed {}
}

pub trait Pixel: private::Sealed + Copy {}

impl private::Sealed for Color {}
impl private::Sealed for RGB   {}
impl Pixel for Color {}
impl Pixel for RGB   {}

#[derive(Debug, Clone)]
pub struct Image<T: Pixel> {
    width:  usize,
    height: usize,
    pixels: Vec<T>,
}

impl<T: Pixel> Image<T> {
    pub fn width(&self)  -> usize {self.width}
    pub fn height(&self) -> usize {self.height}
    pub fn raw(&self)    -> &[T]  {&self.pixels}

    pub fn from_fn<F: Fn(usize, usize) -> T> (width: usize, height: usize, f: F) -> Image<T> {
        let w = StrengthReducedUsize::new(width);

        let pixels = (0..width*height)
            .map(|i| {
                let (y, x) = StrengthReducedUsize::div_rem(i, w);
                f (x, y)
            }).collect();

        Self {width, height, pixels}
    }
}

impl Image<Color> {
    pub fn to_rgb(self) -> Image<RGB> {
        Image {
            width:  self.width,
            height: self.height,
            pixels: self.pixels.into_iter().map(|x| x.to_rgb()).collect()
        }
    }

    pub fn to_rgb_copy(&self) -> Image<RGB> {
        self.clone().to_rgb()
    }
}


