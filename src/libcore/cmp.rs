// NB: transitionary, de-mode-ing.
#[forbid(deprecated_mode)];
#[forbid(deprecated_pattern)];

/// Interfaces used for comparison.

trait Ord {
    pure fn lt(&&other: self) -> bool;
}

trait Eq {
    pure fn eq(&&other: self) -> bool;
}

pure fn lt<T: Ord>(v1: &T, v2: &T) -> bool {
    v1.lt(*v2)
}

pure fn le<T: Ord Eq>(v1: &T, v2: &T) -> bool {
    v1.lt(*v2) || v1.eq(*v2)
}

pure fn eq<T: Eq>(v1: &T, v2: &T) -> bool {
    v1.eq(*v2)
}
