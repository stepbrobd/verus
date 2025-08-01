// rust_verify/tests/example.rs
use verus_builtin_macros::*;
use vstd::*;

verus! {

#[derive(PartialEq, Eq)]
struct Thing {}

#[derive(PartialEq, Eq)]
struct Car {
    thing: Thing,
    four_doors: bool,
}

fn one() {
    let c1 = Car { thing: Thing {  }, four_doors: true };
    let c2 = Car { thing: Thing {  }, four_doors: true };
    assert(c1 == c2);
    let t1 = Thing {  };
    let t2 = Thing {  };
    assert(t1 == t2);
}

fn main() {
}

} // verus!
