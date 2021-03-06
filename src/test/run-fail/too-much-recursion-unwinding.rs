// error-pattern:ran out of stack
// xfail-test - right now we leak when we fail during failure

// Test that the task fails after hiting the recursion limit
// durnig unwinding

fn recurse() {
    log(debug, "don't optimize me out");
    recurse();
}

class r {
  let recursed: *mut bool;
  new(recursed: *mut bool) unsafe { self.recursed = recursed; }
  drop unsafe { 
    if !*recursed {
        *recursed = true;
        recurse();
    }
  }
}

fn main() {
    let mut recursed = false;
    let _r = r(ptr::mut_addr_of(recursed));
    recurse();
}