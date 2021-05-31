pub fn flat_map_all<T, U>(iter: Vec<T>, op: impl Fn(T) -> Option<U>) -> Option<Vec<U>> {
    let mut vs = Vec::with_capacity(iter.len());
    for b in iter {
        vs.push(op(b)?);
    }

    Some(vs)
}
