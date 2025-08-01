use core::marker;

#[allow(unused_imports)]
use super::pervasive::*;
#[allow(unused_imports)]
use super::prelude::*;

verus! {

/// `Seq<A>` is a sequence type for specifications.
/// To use a "sequence" in compiled code, use an `exec` type like `vec::Vec`
/// that has `Seq<A>` as its specification type.
///
/// An object `seq: Seq<A>` has a length, given by [`seq.len()`](Seq::len),
/// and a value at each `i` for `0 <= i < seq.len()`, given by [`seq[i]`](Seq::index).
///
/// Sequences can be constructed in a few different ways:
///  * [`Seq::empty`] construct an empty sequence (`len() == 0`)
///  * [`Seq::new`] construct a sequence of a given length, initialized according
///     to a given function mapping indices `i` to values `A`.
///  * The [`seq!`] macro, to construct small sequences of a fixed size (analagous to the
///     [`std::vec!`] macro).
///  * By manipulating an existing sequence with [`Seq::push`], [`Seq::update`],
///    or [`Seq::add`].
///
/// To prove that two sequences are equal, it is usually easiest to use the
/// extensional equality operator `=~=`.
#[verifier::external_body]
#[verifier::ext_equal]
#[verifier::accept_recursive_types(A)]
pub struct Seq<A> {
    dummy: marker::PhantomData<A>,
}

impl<A> Seq<A> {
    /// An empty sequence (i.e., a sequence of length 0).
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::empty"]
    pub uninterp spec fn empty() -> Seq<A>;

    /// Construct a sequence `s` of length `len` where entry `s[i]` is given by `f(i)`.
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::new"]
    pub uninterp spec fn new(len: nat, f: impl Fn(int) -> A) -> Seq<A>;

    /// The length of a sequence.
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::len"]
    pub uninterp spec fn len(self) -> nat;

    /// Gets the value at the given index `i`.
    ///
    /// If `i` is not in the range `[0, self.len())`, then the resulting value
    /// is meaningless and arbitrary.
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::index"]
    pub uninterp spec fn index(self, i: int) -> A
        recommends
            0 <= i < self.len(),
    ;

    /// `[]` operator, synonymous with `index`
    #[verifier::inline]
    pub open spec fn spec_index(self, i: int) -> A
        recommends
            0 <= i < self.len(),
    {
        self.index(i)
    }

    /// Appends the value `a` to the end of the sequence.
    /// This always increases the length of the sequence by 1.
    /// This often requires annotating the type of the element literal in the sequence,
    /// e.g., `10int`.
    ///
    /// ## Example
    ///
    /// ```rust
    /// proof fn push_test() {
    ///     assert(seq![10int, 11, 12].push(13) =~= seq![10, 11, 12, 13]);
    /// }
    /// ```
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::push"]
    pub uninterp spec fn push(self, a: A) -> Seq<A>;

    /// Updates the sequence at the given index, replacing the element with the given
    /// value, and leaves all other entries unchanged.
    ///
    /// ## Example
    ///
    /// ```rust
    /// proof fn update_test() {
    ///     let s = seq![10, 11, 12, 13, 14];
    ///     let t = s.update(2, -5);
    ///     assert(t =~= seq![10, 11, -5, 13, 14]);
    /// }
    /// ```
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::update"]
    pub uninterp spec fn update(self, i: int, a: A) -> Seq<A>
        recommends
            0 <= i < self.len(),
    ;

    /// Returns a sequence for the given subrange.
    ///
    /// ## Example
    ///
    /// ```rust
    /// proof fn subrange_test() {
    ///     let s = seq![10int, 11, 12, 13, 14];
    ///     //                      ^-------^
    ///     //           0      1   2   3   4   5
    ///     let sub = s.subrange(2, 4);
    ///     assert(sub =~= seq![12, 13]);
    /// }
    /// ```
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::subrange"]
    pub uninterp spec fn subrange(self, start_inclusive: int, end_exclusive: int) -> Seq<A>
        recommends
            0 <= start_inclusive <= end_exclusive <= self.len(),
    ;

    /// Returns a sequence containing only the first n elements of the original sequence
    #[verifier::inline]
    pub open spec fn take(self, n: int) -> Seq<A> {
        self.subrange(0, n)
    }

    /// Returns a sequence without the first n elements of the original sequence
    #[verifier::inline]
    pub open spec fn skip(self, n: int) -> Seq<A> {
        self.subrange(n, self.len() as int)
    }

    /// Concatenates the sequences.
    ///
    /// ## Example
    ///
    /// ```rust
    /// proof fn add_test() {
    ///     assert(seq![10int, 11].add(seq![12, 13, 14])
    ///             =~= seq![10, 11, 12, 13, 14]);
    /// }
    /// ```
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::add"]
    pub uninterp spec fn add(self, rhs: Seq<A>) -> Seq<A>;

    /// `+` operator, synonymous with `add`
    #[verifier::inline]
    pub open spec fn spec_add(self, rhs: Seq<A>) -> Seq<A> {
        self.add(rhs)
    }

    /// Returns the last element of the sequence.
    #[rustc_diagnostic_item = "verus::vstd::seq::Seq::last"]
    pub open spec fn last(self) -> A
        recommends
            0 < self.len(),
    {
        self[self.len() as int - 1]
    }

    /// Returns the first element of the sequence.
    #[rustc_diagnostic_item = "vstd::seq::Seq::first"]
    pub open spec fn first(self) -> A
        recommends
            0 < self.len(),
    {
        self[0]
    }

    #[verifier(external_body)]
    pub proof fn tracked_empty() -> (tracked ret: Self)
        ensures
            ret === Seq::empty(),
    {
        unimplemented!()
    }

    #[verifier(external_body)]
    pub proof fn tracked_remove(tracked &mut self, i: int) -> (tracked ret: A)
        requires
            0 <= i < old(self).len(),
        ensures
            ret === old(self)[i],
            self.len() == old(self).len() - 1,
            self == old(self).remove(i),
    {
        unimplemented!()
    }

    #[verifier(external_body)]
    pub proof fn tracked_insert(tracked &mut self, i: int, tracked v: A)
        requires
            0 <= i <= old(self).len(),
        ensures
            self.len() == old(self).len() + 1,
            self == old(self).insert(i, v),
    {
        unimplemented!()
    }

    #[verifier(external_body)]
    pub proof fn tracked_borrow(tracked &self, i: int) -> (tracked ret: &A)
        requires
            0 <= i < self.len(),
        ensures
            *ret === self[i],
    {
        unimplemented!()
    }

    pub proof fn tracked_push(tracked &mut self, tracked v: A)
        ensures
            *self == old(self).push(v),
            self.len() == old(self).len() + 1,
    {
        broadcast use group_seq_axioms;

        assert(self.insert(self.len() as int, v) =~= self.push(v));
        self.tracked_insert(self.len() as int, v);
    }

    pub proof fn tracked_pop(tracked &mut self) -> (tracked ret: A)
        requires
            old(self).len() > 0,
        ensures
            ret === old(self).last(),
            self.len() == old(self).len() - 1,
            *self == old(self).take(old(self).len() - 1),
    {
        broadcast use group_seq_axioms;

        assert(self.remove(self.len() - 1) =~= self.take(self.len() - 1));
        self.tracked_remove(self.len() - 1)
    }

    pub proof fn tracked_pop_front(tracked &mut self) -> (tracked ret: A)
        requires
            old(self).len() > 0,
        ensures
            ret === old(self).first(),
            self.len() == old(self).len() - 1,
            *self == old(self).drop_first(),
    {
        broadcast use group_seq_axioms;

        assert(self.remove(0) =~= self.drop_first());
        self.tracked_remove(0)
    }
}

// Trusted axioms
pub broadcast axiom fn axiom_seq_index_decreases<A>(s: Seq<A>, i: int)
    requires
        0 <= i < s.len(),
    ensures
        #[trigger] (decreases_to!(s => s[i])),
;

pub axiom fn axiom_seq_len_decreases<A>(s1: Seq<A>, s2: Seq<A>)
    requires
        s2.len() < s1.len(),
        forall|i2: int|
            0 <= i2 < s2.len() && #[trigger] trigger(s2[i2]) ==> exists|i1: int|
                0 <= i1 < s1.len() && s1[i1] == s2[i2],
    ensures
        decreases_to!(s1 => s2),
;

pub broadcast proof fn axiom_seq_subrange_decreases<A>(s: Seq<A>, i: int, j: int)
    requires
        0 <= i <= j <= s.len(),
        s.subrange(i, j).len() < s.len(),
    ensures
        #[trigger] (decreases_to!(s => s.subrange(i, j))),
{
    broadcast use {axiom_seq_subrange_len, axiom_seq_subrange_index};

    let s2 = s.subrange(i, j);
    assert forall|i2: int| 0 <= i2 < s2.len() && #[trigger] trigger(s2[i2]) implies exists|i1: int|
        0 <= i1 < s.len() && s[i1] == s2[i2] by {
        assert(s[i + i2] == s2[i2]);
    }
    axiom_seq_len_decreases(s, s2);
}

pub broadcast axiom fn axiom_seq_empty<A>()
    ensures
        #[trigger] Seq::<A>::empty().len() == 0,
;

pub broadcast axiom fn axiom_seq_new_len<A>(len: nat, f: spec_fn(int) -> A)
    ensures
        #[trigger] Seq::new(len, f).len() == len,
;

pub broadcast axiom fn axiom_seq_new_index<A>(len: nat, f: spec_fn(int) -> A, i: int)
    requires
        0 <= i < len,
    ensures
        #[trigger] Seq::new(len, f)[i] == f(i),
;

pub broadcast axiom fn axiom_seq_push_len<A>(s: Seq<A>, a: A)
    ensures
        #[trigger] s.push(a).len() == s.len() + 1,
;

pub broadcast axiom fn axiom_seq_push_index_same<A>(s: Seq<A>, a: A, i: int)
    requires
        i == s.len(),
    ensures
        #[trigger] s.push(a)[i] == a,
;

pub broadcast axiom fn axiom_seq_push_index_different<A>(s: Seq<A>, a: A, i: int)
    requires
        0 <= i < s.len(),
    ensures
        #[trigger] s.push(a)[i] == s[i],
;

pub broadcast axiom fn axiom_seq_update_len<A>(s: Seq<A>, i: int, a: A)
    requires
        0 <= i < s.len(),
    ensures
        #[trigger] s.update(i, a).len() == s.len(),
;

pub broadcast axiom fn axiom_seq_update_same<A>(s: Seq<A>, i: int, a: A)
    requires
        0 <= i < s.len(),
    ensures
        #[trigger] s.update(i, a)[i] == a,
;

pub broadcast axiom fn axiom_seq_update_different<A>(s: Seq<A>, i1: int, i2: int, a: A)
    requires
        0 <= i1 < s.len(),
        0 <= i2 < s.len(),
        i1 != i2,
    ensures
        #[trigger] s.update(i2, a)[i1] == s[i1],
;

pub broadcast axiom fn axiom_seq_ext_equal<A>(s1: Seq<A>, s2: Seq<A>)
    ensures
        #[trigger] (s1 =~= s2) <==> {
            &&& s1.len() == s2.len()
            &&& forall|i: int| 0 <= i < s1.len() ==> s1[i] == s2[i]
        },
;

pub broadcast axiom fn axiom_seq_ext_equal_deep<A>(s1: Seq<A>, s2: Seq<A>)
    ensures
        #[trigger] (s1 =~~= s2) <==> {
            &&& s1.len() == s2.len()
            &&& forall|i: int| 0 <= i < s1.len() ==> s1[i] =~~= s2[i]
        },
;

pub broadcast axiom fn axiom_seq_subrange_len<A>(s: Seq<A>, j: int, k: int)
    requires
        0 <= j <= k <= s.len(),
    ensures
        #[trigger] s.subrange(j, k).len() == k - j,
;

pub broadcast axiom fn axiom_seq_subrange_index<A>(s: Seq<A>, j: int, k: int, i: int)
    requires
        0 <= j <= k <= s.len(),
        0 <= i < k - j,
    ensures
        #[trigger] s.subrange(j, k)[i] == s[i + j],
;

pub broadcast axiom fn axiom_seq_add_len<A>(s1: Seq<A>, s2: Seq<A>)
    ensures
        #[trigger] s1.add(s2).len() == s1.len() + s2.len(),
;

pub broadcast axiom fn axiom_seq_add_index1<A>(s1: Seq<A>, s2: Seq<A>, i: int)
    requires
        0 <= i < s1.len(),
    ensures
        #[trigger] s1.add(s2)[i] == s1[i],
;

pub broadcast axiom fn axiom_seq_add_index2<A>(s1: Seq<A>, s2: Seq<A>, i: int)
    requires
        s1.len() <= i < s1.len() + s2.len(),
    ensures
        #[trigger] s1.add(s2)[i] == s2[i - s1.len()],
;

pub broadcast group group_seq_axioms {
    axiom_seq_index_decreases,
    axiom_seq_subrange_decreases,
    axiom_seq_empty,
    axiom_seq_new_len,
    axiom_seq_new_index,
    axiom_seq_push_len,
    axiom_seq_push_index_same,
    axiom_seq_push_index_different,
    axiom_seq_update_len,
    axiom_seq_update_same,
    axiom_seq_update_different,
    axiom_seq_ext_equal,
    axiom_seq_ext_equal_deep,
    axiom_seq_subrange_len,
    axiom_seq_subrange_index,
    axiom_seq_add_len,
    axiom_seq_add_index1,
    axiom_seq_add_index2,
}

// ------------- Macros ---------------- //
#[doc(hidden)]
#[macro_export]
macro_rules! seq_internal {
    [] => {
        $crate::vstd::seq::Seq::empty()
    };
    [$elem:expr] => {
        $crate::vstd::seq::Seq::empty()
            .push($elem)
    };
    [$($elem:expr),* $(,)?] => {
        <_ as $crate::vstd::view::View>::view(&[$($elem),*])
    };
    [$elem:expr; $n:expr] => {
        $crate::vstd::seq::Seq::new(
            $n,
            $crate::vstd::prelude::closure_to_fn_spec(
                |_x: _| $elem
            ),
        )
    };
}

/// Creates a [`Seq`] containing the given elements.
///
/// ## Example
///
/// ```rust
/// let s = seq![11int, 12, 13];
///
/// assert(s.len() == 3);
/// assert(s[0] == 11);
/// assert(s[1] == 12);
/// assert(s[2] == 13);
/// ```
#[macro_export]
macro_rules! seq {
    [$($tail:tt)*] => {
        $crate::vstd::prelude::verus_proof_macro_exprs!($crate::vstd::seq::seq_internal!($($tail)*))
    };
}

#[doc(hidden)]
pub use seq_internal;
pub use seq;

} // verus!
