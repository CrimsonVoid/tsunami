pub trait IterExt: Iterator {
    fn flat_map_all<T, U>(self, op: impl Fn(T) -> Option<U>) -> Option<Vec<U>>
    where
        Self: Iterator<Item = T> + Sized,
    {
        let (min, max) = self.size_hint();
        let mut vs = Vec::with_capacity(max.unwrap_or(min));

        for b in self {
            vs.push(op(b)?);
        }

        Some(vs)
    }
}

impl<I: Iterator + Sized> IterExt for I {}
