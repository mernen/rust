mod kitties {

struct cat {
  priv {
    let mut meows : uint;
      fn nap() { for uint::range(1u, 10000u) |_i|{}}
  }

  let how_hungry : int;

  new(in_x : uint, in_y : int) { self.meows = in_x; self.how_hungry = in_y; }
}

}