use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct Vec3<T> (pub T, pub T, pub T);

unsafe impl <T: Pod>      Pod      for Vec3<T> {}
unsafe impl <T: Zeroable> Zeroable for Vec3<T> {}

impl<T> Vec3<T> {
    pub fn map<V, F: Fn(T) -> V> (self, f: F) -> Vec3<V> {
        Vec3(f(self.0), f(self.1), f(self.2))
    }

    pub fn new(a: T, b: T, c: T) -> Vec3<T> {Vec3(a, b, c)}
}
