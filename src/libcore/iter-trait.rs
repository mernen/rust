// This makes use of a clever hack that brson came up with to
// workaround our lack of traits and lack of macros.  See core.{rc,rs} for
// how this file is used.

import inst::{IMPL_T, EACH, SIZE_HINT};
export extensions;

impl<A> IMPL_T<A>: iter::BaseIter<A> {
    pure fn each(blk: fn(A) -> bool) { EACH(self, blk) }
    pure fn size_hint() -> option<uint> { SIZE_HINT(self) }
}

impl<A> IMPL_T<A>: iter::ExtendedIter<A> {
    pure fn eachi(blk: fn(uint, A) -> bool) { iter::eachi(self, blk) }
    pure fn all(blk: fn(A) -> bool) -> bool { iter::all(self, blk) }
    pure fn any(blk: fn(A) -> bool) -> bool { iter::any(self, blk) }
    pure fn foldl<B>(+b0: B, blk: fn(B, A) -> B) -> B {
        iter::foldl(self, b0, blk)
    }
    pure fn contains(x: A) -> bool { iter::contains(self, x) }
    pure fn count(x: A) -> uint { iter::count(self, x) }
    pure fn position(f: fn(A) -> bool) -> option<uint> {
        iter::position(self, f)
    }
}

impl<A: copy> IMPL_T<A>: iter::CopyableIter<A> {
    pure fn filter_to_vec(pred: fn(A) -> bool) -> ~[A] {
        iter::filter_to_vec(self, pred)
    }
    pure fn map_to_vec<B>(op: fn(A) -> B) -> ~[B] {
        iter::map_to_vec(self, op)
    }
    pure fn to_vec() -> ~[A] { iter::to_vec(self) }

    // FIXME--bug in resolve prevents this from working (#2611)
    // fn flat_map_to_vec<B:copy,IB:base_iter<B>>(op: fn(A) -> IB) -> ~[B] {
    //     iter::flat_map_to_vec(self, op)
    // }

    pure fn min() -> A { iter::min(self) }
    pure fn max() -> A { iter::max(self) }
    pure fn find(p: fn(A) -> bool) -> option<A> { iter::find(self, p) }
}
