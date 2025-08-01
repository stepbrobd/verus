#[allow(unused_imports)]
use super::multiset::Multiset;
#[allow(unused_imports)]
use super::pervasive::*;
use super::prelude::Seq;
#[allow(unused_imports)]
use super::prelude::*;
#[allow(unused_imports)]
use super::relations::*;
#[allow(unused_imports)]
use super::set::*;

verus! {

broadcast use super::set::group_set_axioms;

impl<A> Set<A> {
    /// Is `true` if called by a "full" set, i.e., a set containing every element of type `A`.
    pub open spec fn is_full(self) -> bool {
        self == Set::<A>::full()
    }

    /// Is `true` if called by an "empty" set, i.e., a set containing no elements and has length 0
    pub open spec fn is_empty(self) -> (b: bool) {
        self =~= Set::<A>::empty()
    }

    /// Returns the set contains an element `f(x)` for every element `x` in `self`.
    pub open spec fn map<B>(self, f: spec_fn(A) -> B) -> Set<B> {
        Set::new(|a: B| exists|x: A| self.contains(x) && a == f(x))
    }

    /// Converts a set into a sequence with an arbitrary ordering.
    pub open spec fn to_seq(self) -> Seq<A>
        recommends
            self.finite(),
        decreases self.len(),
        when self.finite()
    {
        if self.len() == 0 {
            Seq::<A>::empty()
        } else {
            let x = self.choose();
            Seq::<A>::empty().push(x) + self.remove(x).to_seq()
        }
    }

    /// Converts a set into a sequence sorted by the given ordering function `leq`
    pub open spec fn to_sorted_seq(self, leq: spec_fn(A, A) -> bool) -> Seq<A> {
        self.to_seq().sort_by(leq)
    }

    /// A singleton set has at least one element and any two elements are equal.
    pub open spec fn is_singleton(self) -> bool {
        &&& self.len() > 0
        &&& (forall|x: A, y: A| self.contains(x) && self.contains(y) ==> x == y)
    }

    /// Any totally-ordered set contains a unique minimal (equivalently, least) element.
    /// Returns an arbitrary value if r is not a total ordering
    pub closed spec fn find_unique_minimal(self, r: spec_fn(A, A) -> bool) -> A
        recommends
            total_ordering(r),
            self.len() > 0,
            self.finite(),
        decreases self.len(),
        when self.finite()
    {
        proof {
            broadcast use group_set_properties;

        }
        if self.len() <= 1 {
            self.choose()
        } else {
            let x = choose|x: A| self.contains(x);
            let min = self.remove(x).find_unique_minimal(r);
            if r(min, x) {
                min
            } else {
                x
            }
        }
    }

    /// Proof of correctness and expected behavior for `Set::find_unique_minimal`.
    pub proof fn find_unique_minimal_ensures(self, r: spec_fn(A, A) -> bool)
        requires
            self.finite(),
            self.len() > 0,
            total_ordering(r),
        ensures
            is_minimal(r, self.find_unique_minimal(r), self) && (forall|min: A|
                is_minimal(r, min, self) ==> self.find_unique_minimal(r) == min),
        decreases self.len(),
    {
        broadcast use group_set_properties;

        if self.len() == 1 {
            let x = choose|x: A| self.contains(x);
            assert(self.remove(x).insert(x) =~= self);
            assert(is_minimal(r, self.find_unique_minimal(r), self));
        } else {
            let x = choose|x: A| self.contains(x);
            self.remove(x).find_unique_minimal_ensures(r);
            assert(is_minimal(r, self.remove(x).find_unique_minimal(r), self.remove(x)));
            let y = self.remove(x).find_unique_minimal(r);
            let min_updated = self.find_unique_minimal(r);
            assert(!r(y, x) ==> min_updated == x);
            assert(forall|elt: A|
                self.remove(x).contains(elt) && #[trigger] r(elt, y) ==> #[trigger] r(y, elt));
            assert forall|elt: A|
                self.contains(elt) && #[trigger] r(elt, min_updated) implies #[trigger] r(
                min_updated,
                elt,
            ) by {
                assert(r(min_updated, x) || r(min_updated, y));
                if min_updated == y {  // Case where the new min is the old min
                    assert(is_minimal(r, self.find_unique_minimal(r), self));
                } else {  //Case where the new min is the newest element
                    assert(self.remove(x).contains(elt) || elt == x);
                    assert(min_updated == x);
                    assert(r(x, y) || r(y, x));
                    assert(!r(x, y) || !r(y, x));
                    assert(!(min_updated == y) ==> !r(y, x));
                    assert(r(x, y));
                    if (self.remove(x).contains(elt)) {
                        assert(r(elt, y) && r(y, elt) ==> elt == y);
                    } else {
                    }
                }
            }
            assert forall|min_poss: A|
                is_minimal(r, min_poss, self) implies self.find_unique_minimal(r) == min_poss by {
                assert(is_minimal(r, min_poss, self.remove(x)) || x == min_poss);
                assert(r(min_poss, self.find_unique_minimal(r)));
            }
        }
    }

    /// Any totally-ordered set contains a unique maximal (equivalently, greatest) element.
    /// Returns an arbitrary value if r is not a total ordering
    pub closed spec fn find_unique_maximal(self, r: spec_fn(A, A) -> bool) -> A
        recommends
            total_ordering(r),
            self.len() > 0,
        decreases self.len(),
        when self.finite()
    {
        proof {
            broadcast use group_set_properties;

        }
        if self.len() <= 1 {
            self.choose()
        } else {
            let x = choose|x: A| self.contains(x);
            let max = self.remove(x).find_unique_maximal(r);
            if r(x, max) {
                max
            } else {
                x
            }
        }
    }

    /// Proof of correctness and expected behavior for `Set::find_unique_maximal`.
    pub proof fn find_unique_maximal_ensures(self, r: spec_fn(A, A) -> bool)
        requires
            self.finite(),
            self.len() > 0,
            total_ordering(r),
        ensures
            is_maximal(r, self.find_unique_maximal(r), self) && (forall|max: A|
                is_maximal(r, max, self) ==> self.find_unique_maximal(r) == max),
        decreases self.len(),
    {
        broadcast use group_set_properties;

        if self.len() == 1 {
            let x = choose|x: A| self.contains(x);
            assert(self.remove(x) =~= Set::<A>::empty());
            assert(self.contains(self.find_unique_maximal(r)));
        } else {
            let x = choose|x: A| self.contains(x);
            self.remove(x).find_unique_maximal_ensures(r);
            assert(is_maximal(r, self.remove(x).find_unique_maximal(r), self.remove(x)));
            assert(self.remove(x).insert(x) =~= self);
            let y = self.remove(x).find_unique_maximal(r);
            let max_updated = self.find_unique_maximal(r);
            assert(max_updated == x || max_updated == y);
            assert(!r(x, y) ==> max_updated == x);
            assert forall|elt: A|
                self.contains(elt) && #[trigger] r(max_updated, elt) implies #[trigger] r(
                elt,
                max_updated,
            ) by {
                assert(r(x, max_updated) || r(y, max_updated));
                if max_updated == y {  // Case where the new max is the old max
                    assert(r(elt, max_updated));
                    assert(r(x, max_updated));
                    assert(is_maximal(r, self.find_unique_maximal(r), self));
                } else {  //Case where the new max is the newest element
                    assert(self.remove(x).contains(elt) || elt == x);
                    assert(max_updated == x);
                    assert(r(x, y) || r(y, x));
                    assert(!r(x, y) || !r(y, x));
                    assert(!(max_updated == y) ==> !r(x, y));
                    assert(r(y, x));
                    if (self.remove(x).contains(elt)) {
                        assert(r(y, elt) ==> r(elt, y));
                        assert(r(y, elt) && r(elt, y) ==> elt == y);
                        assert(r(elt, x));
                        assert(r(elt, max_updated))
                    } else {
                    }
                }
            }
            assert forall|max_poss: A|
                is_maximal(r, max_poss, self) implies self.find_unique_maximal(r) == max_poss by {
                assert(is_maximal(r, max_poss, self.remove(x)) || x == max_poss);
                assert(r(max_poss, self.find_unique_maximal(r)));
                assert(r(self.find_unique_maximal(r), max_poss));
            }
        }
    }

    /// Converts a set into a multiset where each element from the set has
    /// multiplicity 1 and any other element has multiplicity 0.
    pub open spec fn to_multiset(self) -> Multiset<A>
        decreases self.len(),
        when self.finite()
    {
        if self.len() == 0 {
            Multiset::<A>::empty()
        } else {
            Multiset::<A>::empty().insert(self.choose()).add(
                self.remove(self.choose()).to_multiset(),
            )
        }
    }

    /// A finite set with length 0 is equivalent to the empty set.
    pub proof fn lemma_len0_is_empty(self)
        requires
            self.finite(),
            self.len() == 0,
        ensures
            self == Set::<A>::empty(),
    {
        if exists|a: A| self.contains(a) {
            // derive contradiction:
            assert(self.remove(self.choose()).len() + 1 == 0);
        }
        assert(self =~= Set::empty());
    }

    /// A singleton set has length 1.
    pub proof fn lemma_singleton_size(self)
        requires
            self.is_singleton(),
        ensures
            self.len() == 1,
    {
        broadcast use group_set_properties;

        assert(self.remove(self.choose()) =~= Set::empty());
    }

    /// A set has exactly one element, if and only if, it has at least one element and any two elements are equal.
    pub proof fn lemma_is_singleton(s: Set<A>)
        requires
            s.finite(),
        ensures
            s.is_singleton() == (s.len() == 1),
    {
        if s.is_singleton() {
            s.lemma_singleton_size();
        }
        if s.len() == 1 {
            assert forall|x: A, y: A| s.contains(x) && s.contains(y) implies x == y by {
                let x = choose|x: A| s.contains(x);
                broadcast use group_set_properties;

                assert(s.remove(x).len() == 0);
                assert(s.insert(x) =~= s);
            }
        }
    }

    /// The result of filtering a finite set is finite and has size less than or equal to the original set.
    pub proof fn lemma_len_filter(self, f: spec_fn(A) -> bool)
        requires
            self.finite(),
        ensures
            self.filter(f).finite(),
            self.filter(f).len() <= self.len(),
        decreases self.len(),
    {
        lemma_len_intersect::<A>(self, Set::new(f));
    }

    /// In a pre-ordered set, a greatest element is necessarily maximal.
    pub proof fn lemma_greatest_implies_maximal(self, r: spec_fn(A, A) -> bool, max: A)
        requires
            pre_ordering(r),
        ensures
            is_greatest(r, max, self) ==> is_maximal(r, max, self),
    {
    }

    /// In a pre-ordered set, a least element is necessarily minimal.
    pub proof fn lemma_least_implies_minimal(self, r: spec_fn(A, A) -> bool, min: A)
        requires
            pre_ordering(r),
        ensures
            is_least(r, min, self) ==> is_minimal(r, min, self),
    {
    }

    /// In a totally-ordered set, an element is maximal if and only if it is a greatest element.
    pub proof fn lemma_maximal_equivalent_greatest(self, r: spec_fn(A, A) -> bool, max: A)
        requires
            total_ordering(r),
        ensures
            is_greatest(r, max, self) <==> is_maximal(r, max, self),
    {
        assert(is_maximal(r, max, self) ==> forall|x: A|
            !self.contains(x) || !r(max, x) || r(x, max));
    }

    /// In a totally-ordered set, an element is maximal if and only if it is a greatest element.
    pub proof fn lemma_minimal_equivalent_least(self, r: spec_fn(A, A) -> bool, min: A)
        requires
            total_ordering(r),
        ensures
            is_least(r, min, self) <==> is_minimal(r, min, self),
    {
        assert(is_minimal(r, min, self) ==> forall|x: A|
            !self.contains(x) || !r(x, min) || r(min, x));
    }

    /// In a partially-ordered set, there exists at most one least element.
    pub proof fn lemma_least_is_unique(self, r: spec_fn(A, A) -> bool)
        requires
            partial_ordering(r),
        ensures
            forall|min: A, min_prime: A|
                is_least(r, min, self) && is_least(r, min_prime, self) ==> min == min_prime,
    {
        assert forall|min: A, min_prime: A|
            is_least(r, min, self) && is_least(r, min_prime, self) implies min == min_prime by {
            assert(r(min, min_prime));
            assert(r(min_prime, min));
        }
    }

    /// In a partially-ordered set, there exists at most one greatest element.
    pub proof fn lemma_greatest_is_unique(self, r: spec_fn(A, A) -> bool)
        requires
            partial_ordering(r),
        ensures
            forall|max: A, max_prime: A|
                is_greatest(r, max, self) && is_greatest(r, max_prime, self) ==> max == max_prime,
    {
        assert forall|max: A, max_prime: A|
            is_greatest(r, max, self) && is_greatest(r, max_prime, self) implies max
            == max_prime by {
            assert(r(max_prime, max));
            assert(r(max, max_prime));
        }
    }

    /// In a totally-ordered set, there exists at most one minimal element.
    pub proof fn lemma_minimal_is_unique(self, r: spec_fn(A, A) -> bool)
        requires
            total_ordering(r),
        ensures
            forall|min: A, min_prime: A|
                is_minimal(r, min, self) && is_minimal(r, min_prime, self) ==> min == min_prime,
    {
        assert forall|min: A, min_prime: A|
            is_minimal(r, min, self) && is_minimal(r, min_prime, self) implies min == min_prime by {
            self.lemma_minimal_equivalent_least(r, min);
            self.lemma_minimal_equivalent_least(r, min_prime);
            self.lemma_least_is_unique(r);
        }
    }

    /// In a totally-ordered set, there exists at most one maximal element.
    pub proof fn lemma_maximal_is_unique(self, r: spec_fn(A, A) -> bool)
        requires
            self.finite(),
            total_ordering(r),
        ensures
            forall|max: A, max_prime: A|
                is_maximal(r, max, self) && is_maximal(r, max_prime, self) ==> max == max_prime,
    {
        assert forall|max: A, max_prime: A|
            is_maximal(r, max, self) && is_maximal(r, max_prime, self) implies max == max_prime by {
            self.lemma_maximal_equivalent_greatest(r, max);
            self.lemma_maximal_equivalent_greatest(r, max_prime);
            self.lemma_greatest_is_unique(r);
        }
    }

    /// Set difference with an additional element inserted decreases the size of
    /// the result. This can be useful for proving termination when traversing
    /// a set while tracking the elements that have already been handled.
    pub broadcast proof fn lemma_set_insert_diff_decreases(self, s: Set<A>, elt: A)
        requires
            self.contains(elt),
            !s.contains(elt),
            self.finite(),
        ensures
            #[trigger] self.difference(s.insert(elt)).len() < self.difference(s).len(),
    {
        self.difference(s.insert(elt)).lemma_subset_not_in_lt(self.difference(s), elt);
    }

    /// If there is an element not present in a subset, its length is stricly smaller.
    pub proof fn lemma_subset_not_in_lt(self: Set<A>, s2: Set<A>, elt: A)
        requires
            self.subset_of(s2),
            s2.finite(),
            !self.contains(elt),
            s2.contains(elt),
        ensures
            self.len() < s2.len(),
    {
        let s2_no_elt = s2.remove(elt);
        assert(self.len() <= s2_no_elt.len()) by {
            lemma_len_subset(self, s2_no_elt);
        }
    }

    /// Inserting an element and mapping a function over a set commute
    pub broadcast proof fn lemma_set_map_insert_commute<B>(self, elt: A, f: spec_fn(A) -> B)
        ensures
            #[trigger] self.insert(elt).map(f) =~= self.map(f).insert(f(elt)),
    {
        assert forall|x: B| self.map(f).insert(f(elt)).contains(x) implies self.insert(elt).map(
            f,
        ).contains(x) by {
            if x == f(elt) {
                assert(self.insert(elt).contains(elt));
            } else {
                let y = choose|y: A| self.contains(y) && f(y) == x;
                assert(self.insert(elt).contains(y));
            }
        }
    }

    /// `map` and `union` commute
    pub proof fn lemma_map_union_commute<B>(self, t: Set<A>, f: spec_fn(A) -> B)
        ensures
            (self.union(t)).map(f) =~= self.map(f).union(t.map(f)),
    {
        broadcast use group_set_axioms;

        let lhs = self.union(t).map(f);
        let rhs = self.map(f).union(t.map(f));

        assert forall|elem: B| rhs.contains(elem) implies lhs.contains(elem) by {
            if self.map(f).contains(elem) {
                let preimage = choose|preimage: A| self.contains(preimage) && f(preimage) == elem;
                assert(self.union(t).contains(preimage));
            } else {
                assert(t.map(f).contains(elem));
                let preimage = choose|preimage: A| t.contains(preimage) && f(preimage) == elem;
                assert(self.union(t).contains(preimage));
            }
        }
    }

    /// Utility function for more concise universal quantification over sets
    pub open spec fn all(&self, pred: spec_fn(A) -> bool) -> bool {
        forall|x: A| self.contains(x) ==> pred(x)
    }

    /// Utility function for more concise existential quantification over sets
    pub open spec fn any(&self, pred: spec_fn(A) -> bool) -> bool {
        exists|x: A| self.contains(x) && pred(x)
    }

    /// `any` is preserved between predicates `p` and `q` if `p` implies `q`.
    pub broadcast proof fn lemma_any_map_preserved_pred<B>(
        self,
        p: spec_fn(A) -> bool,
        q: spec_fn(B) -> bool,
        f: spec_fn(A) -> B,
    )
        requires
            #[trigger] self.any(p),
            forall|x: A| #[trigger] p(x) ==> q(f(x)),
        ensures
            #[trigger] self.map(f).any(q),
    {
        let x = choose|x: A| self.contains(x) && p(x);
        assert(self.map(f).contains(f(x)));
    }

    /// Collecting all elements `b` where `f` returns `Some(b)`
    pub open spec fn filter_map<B>(self, f: spec_fn(A) -> Option<B>) -> Set<B> {
        self.map(
            |elem: A|
                match f(elem) {
                    Option::Some(r) => set!{r},
                    Option::None => set!{},
                },
        ).flatten()
    }

    /// Inserting commutes with `filter_map`
    pub broadcast proof fn lemma_filter_map_insert<B>(
        s: Set<A>,
        f: spec_fn(A) -> Option<B>,
        elem: A,
    )
        ensures
            #[trigger] s.insert(elem).filter_map(f) == (match f(elem) {
                Some(res) => s.filter_map(f).insert(res),
                None => s.filter_map(f),
            }),
    {
        broadcast use group_set_axioms;
        broadcast use Set::lemma_set_map_insert_commute;

        let lhs = s.insert(elem).filter_map(f);
        let rhs = match f(elem) {
            Some(res) => s.filter_map(f).insert(res),
            None => s.filter_map(f),
        };
        let to_set = |elem: A|
            match f(elem) {
                Option::Some(r) => set!{r},
                Option::None => set!{},
            };
        assert forall|r: B| #[trigger] lhs.contains(r) implies rhs.contains(r) by {
            if f(elem) != Some(r) {
                let orig = choose|orig: A| #[trigger]
                    s.contains(orig) && f(orig) == Option::Some(r);
                assert(to_set(orig) == set!{r});
                assert(s.map(to_set).contains(to_set(orig)));
            }
        }
        assert forall|r: B| #[trigger] rhs.contains(r) implies lhs.contains(r) by {
            if Some(r) == f(elem) {
                assert(s.insert(elem).map(to_set).contains(to_set(elem)));
            } else {
                let orig = choose|orig: A| #[trigger]
                    s.contains(orig) && f(orig) == Option::Some(r);
                assert(s.insert(elem).map(to_set).contains(to_set(orig)));
            }
        }
        assert(lhs =~= rhs);
    }

    /// `filter_map` and `union` commute.
    pub broadcast proof fn lemma_filter_map_union<B>(self, f: spec_fn(A) -> Option<B>, t: Set<A>)
        ensures
            #[trigger] self.union(t).filter_map(f) == self.filter_map(f).union(t.filter_map(f)),
    {
        broadcast use group_set_axioms;

        let lhs = self.union(t).filter_map(f);
        let rhs = self.filter_map(f).union(t.filter_map(f));
        let to_set = |elem: A|
            match f(elem) {
                Option::Some(r) => set!{r},
                Option::None => set!{},
            };

        assert forall|elem: B| rhs.contains(elem) implies lhs.contains(elem) by {
            if self.filter_map(f).contains(elem) {
                let x = choose|x: A| self.contains(x) && f(x) == Option::Some(elem);
                assert(self.union(t).contains(x));
                assert(self.union(t).map(to_set).contains(to_set(x)));
            }
            if t.filter_map(f).contains(elem) {
                let x = choose|x: A| t.contains(x) && f(x) == Option::Some(elem);
                assert(self.union(t).contains(x));
                assert(self.union(t).map(to_set).contains(to_set(x)));
            }
        }
        assert forall|elem: B| lhs.contains(elem) implies rhs.contains(elem) by {
            let x = choose|x: A| self.union(t).contains(x) && f(x) == Option::Some(elem);
            if self.contains(x) {
                assert(self.map(to_set).contains(to_set(x)));
                assert(self.filter_map(f).contains(elem));
            } else {
                assert(t.contains(x));
                assert(t.map(to_set).contains(to_set(x)));
                assert(t.filter_map(f).contains(elem));
            }
        }
        assert(lhs =~= rhs);
    }

    /// `map` preserves finiteness
    pub proof fn lemma_map_finite<B>(self, f: spec_fn(A) -> B)
        requires
            self.finite(),
        ensures
            self.map(f).finite(),
        decreases self.len(),
    {
        broadcast use group_set_axioms;
        broadcast use lemma_set_empty_equivalency_len;

        if self.len() == 0 {
            assert(forall|elem: A| !(#[trigger] self.contains(elem)));
            assert forall|res: B| #[trigger] self.map(f).contains(res) implies false by {
                let x = choose|x: A| self.contains(x) && f(x) == res;
            }
            assert(self.map(f) =~= Set::<B>::empty());
        } else {
            let x = choose|x: A| self.contains(x);
            assert(self.map(f).contains(f(x)));
            self.remove(x).lemma_map_finite(f);
            assert(self.remove(x).insert(x) == self);
            assert(self.map(f) == self.remove(x).map(f).insert(f(x)));
        }
    }

    pub broadcast proof fn lemma_set_all_subset(self, s2: Set<A>, p: spec_fn(A) -> bool)
        requires
            #[trigger] self.subset_of(s2),
            s2.all(p),
        ensures
            #[trigger] self.all(p),
    {
        broadcast use group_set_axioms;

    }

    /// `filter_map` preserves finiteness.
    pub broadcast proof fn lemma_filter_map_finite<B>(self, f: spec_fn(A) -> Option<B>)
        requires
            self.finite(),
        ensures
            #[trigger] self.filter_map(f).finite(),
        decreases self.len(),
    {
        broadcast use group_set_axioms;
        broadcast use Set::lemma_filter_map_insert;

        let mapped = self.filter_map(f);
        if self.len() == 0 {
            assert(self.filter_map(f) =~= Set::<B>::empty());
        } else {
            let elem = self.choose();
            self.remove(elem).lemma_filter_map_finite(f);
            assert(self =~= self.remove(elem).insert(elem));
        }
    }

    /// Conversion to a sequence and back to a set is the identity function.
    pub broadcast proof fn lemma_to_seq_to_set_id(self)
        requires
            self.finite(),
        ensures
            #[trigger] self.to_seq().to_set() =~= self,
        decreases self.len(),
    {
        broadcast use group_set_axioms;
        broadcast use lemma_set_empty_equivalency_len;
        broadcast use super::seq_lib::group_seq_properties;

        if self.len() == 0 {
            assert(self.to_seq().to_set() =~= Set::<A>::empty());
        } else {
            let elem = self.choose();
            self.remove(elem).lemma_to_seq_to_set_id();
            assert(self =~= self.remove(elem).insert(elem));
            assert(self.to_seq().to_set() =~= self.remove(elem).to_seq().to_set().insert(elem));
        }
    }
}

impl<A> Set<Set<A>> {
    pub open spec fn flatten(self) -> Set<A> {
        Set::new(
            |elem| exists|elem_s: Set<A>| #[trigger] self.contains(elem_s) && elem_s.contains(elem),
        )
    }

    pub broadcast proof fn flatten_insert_union_commute(self, other: Set<A>)
        ensures
            self.flatten().union(other) =~= #[trigger] self.insert(other).flatten(),
    {
        broadcast use group_set_axioms;

        let lhs = self.flatten().union(other);
        let rhs = self.insert(other).flatten();

        assert forall|elem: A| lhs.contains(elem) implies rhs.contains(elem) by {
            if exists|s: Set<A>| self.contains(s) && s.contains(elem) {
                let s = choose|s: Set<A>| self.contains(s) && s.contains(elem);
                assert(self.insert(other).contains(s));
                assert(s.contains(elem));
            } else {
                assert(self.insert(other).contains(other));
            }
        }
    }
}

/// Two sets are equal iff mapping `f` results in equal sets, if `f` is injective.
pub proof fn lemma_sets_eq_iff_injective_map_eq<T, S>(s1: Set<T>, s2: Set<T>, f: spec_fn(T) -> S)
    requires
        super::relations::injective(f),
    ensures
        (s1 == s2) <==> (s1.map(f) == s2.map(f)),
{
    broadcast use group_set_axioms;

    if (s1.map(f) == s2.map(f)) {
        assert(s1.map(f).len() == s2.map(f).len());
        if !s1.subset_of(s2) {
            let x = choose|x: T| s1.contains(x) && !s2.contains(x);
            assert(s1.map(f).contains(f(x)));
        } else if !s2.subset_of(s1) {
            let x = choose|x: T| s2.contains(x) && !s1.contains(x);
            assert(s2.map(f).contains(f(x)));
        }
        assert(s1 =~= s2);
    }
}

/// The result of inserting an element `a` into a set `s` is finite iff `s` is finite.
pub broadcast proof fn lemma_set_insert_finite_iff<A>(s: Set<A>, a: A)
    ensures
        #[trigger] s.insert(a).finite() <==> s.finite(),
{
    if s.insert(a).finite() {
        if s.contains(a) {
            assert(s == s.insert(a));
        } else {
            assert(s == s.insert(a).remove(a));
        }
    }
    assert(s.insert(a).finite() ==> s.finite());
}

/// The result of removing an element `a` into a set `s` is finite iff `s` is finite.
pub broadcast proof fn lemma_set_remove_finite_iff<A>(s: Set<A>, a: A)
    ensures
        #[trigger] s.remove(a).finite() <==> s.finite(),
{
    if s.remove(a).finite() {
        if s.contains(a) {
            assert(s == s.remove(a).insert(a));
        } else {
            assert(s == s.remove(a));
        }
    }
}

/// The union of two sets is finite iff both sets are finite.
pub broadcast proof fn lemma_set_union_finite_iff<A>(s1: Set<A>, s2: Set<A>)
    ensures
        #[trigger] s1.union(s2).finite() <==> s1.finite() && s2.finite(),
{
    if s1.union(s2).finite() {
        lemma_set_union_finite_implies_sets_finite(s1, s2);
    }
}

pub proof fn lemma_set_union_finite_implies_sets_finite<A>(s1: Set<A>, s2: Set<A>)
    requires
        s1.union(s2).finite(),
    ensures
        s1.finite(),
        s2.finite(),
    decreases s1.union(s2).len(),
{
    if s1.union(s2) =~= set![] {
        assert(s1 =~= set![]);
        assert(s2 =~= set![]);
    } else {
        let a = s1.union(s2).choose();
        assert(s1.remove(a).union(s2.remove(a)) == s1.union(s2).remove(a));
        axiom_set_remove_len(s1.union(s2), a);
        lemma_set_union_finite_implies_sets_finite(s1.remove(a), s2.remove(a));
        assert(forall|s: Set<A>|
            #![auto]
            s.remove(a).insert(a) == if s.contains(a) {
                s
            } else {
                s.insert(a)
            });
        lemma_set_insert_finite_iff(s1, a);
        lemma_set_insert_finite_iff(s2, a);
    }
}

/// The size of a union of two sets is less than or equal to the size of
/// both individual sets combined.
pub proof fn lemma_len_union<A>(s1: Set<A>, s2: Set<A>)
    requires
        s1.finite(),
        s2.finite(),
    ensures
        s1.union(s2).len() <= s1.len() + s2.len(),
    decreases s1.len(),
{
    if s1.is_empty() {
        assert(s1.union(s2) =~= s2);
    } else {
        let a = s1.choose();
        if s2.contains(a) {
            assert(s1.union(s2) =~= s1.remove(a).union(s2));
        } else {
            assert(s1.union(s2).remove(a) =~= s1.remove(a).union(s2));
        }
        lemma_len_union::<A>(s1.remove(a), s2);
    }
}

/// The size of a union of two sets is greater than or equal to the size of
/// both individual sets.
pub proof fn lemma_len_union_ind<A>(s1: Set<A>, s2: Set<A>)
    requires
        s1.finite(),
        s2.finite(),
    ensures
        s1.union(s2).len() >= s1.len(),
        s1.union(s2).len() >= s2.len(),
    decreases s2.len(),
{
    broadcast use group_set_properties;

    if s2.len() == 0 {
    } else {
        let y = choose|y: A| s2.contains(y);
        if s1.contains(y) {
            assert(s1.remove(y).union(s2.remove(y)) =~= s1.union(s2).remove(y));
            lemma_len_union_ind(s1.remove(y), s2.remove(y))
        } else {
            assert(s1.union(s2.remove(y)) =~= s1.union(s2).remove(y));
            lemma_len_union_ind(s1, s2.remove(y))
        }
    }
}

/// The size of the intersection of finite set `s1` and set `s2` is less than or equal to the size of `s1`.
pub proof fn lemma_len_intersect<A>(s1: Set<A>, s2: Set<A>)
    requires
        s1.finite(),
    ensures
        s1.intersect(s2).len() <= s1.len(),
    decreases s1.len(),
{
    if s1.is_empty() {
        assert(s1.intersect(s2) =~= s1);
    } else {
        let a = s1.choose();
        assert(s1.intersect(s2).remove(a) =~= s1.remove(a).intersect(s2));
        lemma_len_intersect::<A>(s1.remove(a), s2);
    }
}

/// If `s1` is a subset of finite set `s2`, then the size of `s1` is less than or equal to
/// the size of `s2` and `s1` must be finite.
pub proof fn lemma_len_subset<A>(s1: Set<A>, s2: Set<A>)
    requires
        s2.finite(),
        s1.subset_of(s2),
    ensures
        s1.len() <= s2.len(),
        s1.finite(),
{
    lemma_len_intersect::<A>(s2, s1);
    assert(s2.intersect(s1) =~= s1);
}

/// A subset of a finite set `s` is finite.
pub broadcast proof fn lemma_set_subset_finite<A>(s: Set<A>, sub: Set<A>)
    requires
        s.finite(),
        sub.subset_of(s),
    ensures
        #![trigger sub.subset_of(s)]
        sub.finite(),
{
    let complement = s.difference(sub);
    assert(sub =~= s.difference(complement));
}

/// The size of the difference of finite set `s1` and set `s2` is less than or equal to the size of `s1`.
pub proof fn lemma_len_difference<A>(s1: Set<A>, s2: Set<A>)
    requires
        s1.finite(),
    ensures
        s1.difference(s2).len() <= s1.len(),
    decreases s1.len(),
{
    if s1.is_empty() {
        assert(s1.difference(s2) =~= s1);
    } else {
        let a = s1.choose();
        assert(s1.difference(s2).remove(a) =~= s1.remove(a).difference(s2));
        lemma_len_difference::<A>(s1.remove(a), s2);
    }
}

/// Creates a finite set of integers in the range [lo, hi).
pub open spec fn set_int_range(lo: int, hi: int) -> Set<int> {
    Set::new(|i: int| lo <= i && i < hi)
}

/// If a set solely contains integers in the range [a, b), then its size is
/// bounded by b - a.
pub proof fn lemma_int_range(lo: int, hi: int)
    requires
        lo <= hi,
    ensures
        set_int_range(lo, hi).finite(),
        set_int_range(lo, hi).len() == hi - lo,
    decreases hi - lo,
{
    if lo == hi {
        assert(set_int_range(lo, hi) =~= Set::empty());
    } else {
        lemma_int_range(lo, hi - 1);
        assert(set_int_range(lo, hi - 1).insert(hi - 1) =~= set_int_range(lo, hi));
    }
}

/// If x is a subset of y and the size of x is equal to the size of y, x is equal to y.
pub proof fn lemma_subset_equality<A>(x: Set<A>, y: Set<A>)
    requires
        x.subset_of(y),
        x.finite(),
        y.finite(),
        x.len() == y.len(),
    ensures
        x =~= y,
    decreases x.len(),
{
    broadcast use group_set_properties;

    if x =~= Set::<A>::empty() {
    } else {
        let e = x.choose();
        lemma_subset_equality(x.remove(e), y.remove(e));
    }
}

/// If an injective function is applied to each element of a set to construct
/// another set, the two sets have the same size.
// the dafny original lemma reasons with partial function f
pub proof fn lemma_map_size<A, B>(x: Set<A>, y: Set<B>, f: spec_fn(A) -> B)
    requires
        injective(f),
        forall|a: A| x.contains(a) ==> y.contains(#[trigger] f(a)),
        forall|b: B| (#[trigger] y.contains(b)) ==> exists|a: A| x.contains(a) && f(a) == b,
        x.finite(),
        y.finite(),
    ensures
        x.len() == y.len(),
    decreases x.len(),
{
    broadcast use group_set_properties;

    if x.len() != 0 {
        let a = x.choose();
        lemma_map_size(x.remove(a), y.remove(f(a)), f);
    }
}

// This verified lemma used to be an axiom in the Dafny prelude
/// Taking the union of sets `a` and `b` and then taking the union of the result with `b`
/// is the same as taking the union of `a` and `b` once.
pub broadcast proof fn lemma_set_union_again1<A>(a: Set<A>, b: Set<A>)
    ensures
        #[trigger] a.union(b).union(b) =~= a.union(b),
{
}

// This verified lemma used to be an axiom in the Dafny prelude
/// Taking the union of sets `a` and `b` and then taking the union of the result with `a`
/// is the same as taking the union of `a` and `b` once.
pub broadcast proof fn lemma_set_union_again2<A>(a: Set<A>, b: Set<A>)
    ensures
        #[trigger] a.union(b).union(a) =~= a.union(b),
{
}

// This verified lemma used to be an axiom in the Dafny prelude
/// Taking the intersection of sets `a` and `b` and then taking the intersection of the result with `b`
/// is the same as taking the intersection of `a` and `b` once.
pub broadcast proof fn lemma_set_intersect_again1<A>(a: Set<A>, b: Set<A>)
    ensures
        #![trigger (a.intersect(b)).intersect(b)]
        (a.intersect(b)).intersect(b) =~= a.intersect(b),
{
}

// This verified lemma used to be an axiom in the Dafny prelude
/// Taking the intersection of sets `a` and `b` and then taking the intersection of the result with `a`
/// is the same as taking the intersection of `a` and `b` once.
pub broadcast proof fn lemma_set_intersect_again2<A>(a: Set<A>, b: Set<A>)
    ensures
        #![trigger (a.intersect(b)).intersect(a)]
        (a.intersect(b)).intersect(a) =~= a.intersect(b),
{
}

// This verified lemma used to be an axiom in the Dafny prelude
/// If set `s2` contains element `a`, then the set difference of `s1` and `s2` does not contain `a`.
pub broadcast proof fn lemma_set_difference2<A>(s1: Set<A>, s2: Set<A>, a: A)
    ensures
        #![trigger s1.difference(s2).contains(a)]
        s2.contains(a) ==> !s1.difference(s2).contains(a),
{
}

// This verified lemma used to be an axiom in the Dafny prelude
/// If sets `a` and `b` are disjoint, meaning they have no elements in common, then the set difference
/// of `a + b` and `b` is equal to `a` and the set difference of `a + b` and `a` is equal to `b`.
pub broadcast proof fn lemma_set_disjoint<A>(a: Set<A>, b: Set<A>)
    ensures
        #![trigger (a + b).difference(a)]  //TODO: this might be too free
        a.disjoint(b) ==> ((a + b).difference(a) =~= b && (a + b).difference(b) =~= a),
{
}

// This verified lemma used to be an axiom in the Dafny prelude
// Dafny encodes the second clause with a single directional, although
// it should be fine with both directions?
// REVIEW: excluded from broadcast group if trigger is too free
//         also not that some proofs in seq_lib requires this lemma
/// Set `s` has length 0 if and only if it is equal to the empty set. If `s` has length greater than 0,
/// Then there must exist an element `x` such that `s` contains `x`.
pub broadcast proof fn lemma_set_empty_equivalency_len<A>(s: Set<A>)
    requires
        s.finite(),
    ensures
        #![trigger s.len()]
        (s.len() == 0 <==> s == Set::<A>::empty()) && (s.len() != 0 ==> exists|x: A| s.contains(x)),
{
    assert(s.len() == 0 ==> s =~= Set::empty()) by {
        if s.len() == 0 {
            assert(forall|a: A| !(Set::empty().contains(a)));
            assert(Set::<A>::empty().len() == 0);
            assert(Set::<A>::empty().len() == s.len());
            assert((exists|a: A| s.contains(a)) || (forall|a: A| !s.contains(a)));
            if exists|a: A| s.contains(a) {
                let a = s.choose();
                assert(s.remove(a).len() == s.len() - 1) by {
                    axiom_set_remove_len(s, a);
                }
            }
        }
    }
    assert(s.len() == 0 <== s =~= Set::empty());
}

// This verified lemma used to be an axiom in the Dafny prelude
/// If sets `a` and `b` are disjoint, meaning they share no elements in common, then the length
/// of the union `a + b` is equal to the sum of the lengths of `a` and `b`.
pub broadcast proof fn lemma_set_disjoint_lens<A>(a: Set<A>, b: Set<A>)
    requires
        a.finite(),
        b.finite(),
    ensures
        a.disjoint(b) ==> #[trigger] (a + b).len() == a.len() + b.len(),
    decreases a.len(),
{
    if a.len() == 0 {
        lemma_set_empty_equivalency_len(a);
        assert(a + b =~= b);
    } else {
        if a.disjoint(b) {
            let x = a.choose();
            assert(a.remove(x) + b =~= (a + b).remove(x));
            lemma_set_disjoint_lens(a.remove(x), b);
        }
    }
}

// This verified lemma used to be an axiom in the Dafny prelude
/// The length of the union between two sets added to the length of the intersection between the
/// two sets is equal to the sum of the lengths of the two sets.
pub broadcast proof fn lemma_set_intersect_union_lens<A>(a: Set<A>, b: Set<A>)
    requires
        a.finite(),
        b.finite(),
    ensures
        #[trigger] (a + b).len() + #[trigger] a.intersect(b).len() == a.len() + b.len(),
    decreases a.len(),
{
    if a.len() == 0 {
        lemma_set_empty_equivalency_len(a);
        assert(a + b =~= b);
        assert(a.intersect(b) =~= Set::empty());
        assert(a.intersect(b).len() == 0);
    } else {
        let x = a.choose();
        lemma_set_intersect_union_lens(a.remove(x), b);
        if (b.contains(x)) {
            assert(a.remove(x) + b =~= (a + b));
            assert(a.intersect(b).remove(x) =~= a.remove(x).intersect(b));
        } else {
            assert(a.remove(x) + b =~= (a + b).remove(x));
            assert(a.remove(x).intersect(b) =~= a.intersect(b));
        }
    }
}

// This verified lemma used to be an axiom in the Dafny prelude
/// The length of the set difference `A \ B` added to the length of the set difference `B \ A` added to
/// the length of the intersection `A ∩ B` is equal to the length of the union `A + B`.
///
/// The length of the set difference `A \ B` is equal to the length of `A` minus the length of the
/// intersection `A ∩ B`.
pub broadcast proof fn lemma_set_difference_len<A>(a: Set<A>, b: Set<A>)
    requires
        a.finite(),
        b.finite(),
    ensures
        (#[trigger] a.difference(b).len() + b.difference(a).len() + a.intersect(b).len() == (a
            + b).len()) && (a.difference(b).len() == a.len() - a.intersect(b).len()),
    decreases a.len(),
{
    if a.len() == 0 {
        lemma_set_empty_equivalency_len(a);
        assert(a.difference(b) =~= Set::empty());
        assert(b.difference(a) =~= b);
        assert(a.intersect(b) =~= Set::empty());
        assert(a + b =~= b);
    } else {
        let x = a.choose();
        lemma_set_difference_len(a.remove(x), b);
        if b.contains(x) {
            assert(a.intersect(b).remove(x) =~= a.remove(x).intersect(b));
            assert(a.remove(x).difference(b) =~= a.difference(b));
            assert(b.difference(a.remove(x)).remove(x) =~= b.difference(a));
            assert(a.remove(x) + b =~= a + b);
        } else {
            assert(a.remove(x) + b =~= (a + b).remove(x));
            assert(a.remove(x).difference(b) =~= a.difference(b).remove(x));
            assert(b.difference(a.remove(x)) =~= b.difference(a));
            assert(a.remove(x).intersect(b) =~= a.intersect(b));
        }
    }
}

/// Properties of sets from the Dafny prelude (which were axioms in Dafny, but proven here in Verus)
#[deprecated = "Use `broadcast use group_set_properties` instead"]
pub proof fn lemma_set_properties<A>()
    ensures
        forall|a: Set<A>, b: Set<A>| #[trigger] a.union(b).union(b) == a.union(b),  //from lemma_set_union_again1
        forall|a: Set<A>, b: Set<A>| #[trigger] a.union(b).union(a) == a.union(b),  //from lemma_set_union_again2
        forall|a: Set<A>, b: Set<A>| #[trigger] (a.intersect(b)).intersect(b) == a.intersect(b),  //from lemma_set_intersect_again1
        forall|a: Set<A>, b: Set<A>| #[trigger] (a.intersect(b)).intersect(a) == a.intersect(b),  //from lemma_set_intersect_again2
        forall|s1: Set<A>, s2: Set<A>, a: A| s2.contains(a) ==> !s1.difference(s2).contains(a),  //from lemma_set_difference2
        forall|a: Set<A>, b: Set<A>|
            #![trigger (a + b).difference(a)]
            a.disjoint(b) ==> ((a + b).difference(a) =~= b && (a + b).difference(b) =~= a),  //from lemma_set_disjoint
        forall|s: Set<A>| #[trigger] s.len() != 0 && s.finite() ==> exists|a: A| s.contains(a),  // half of lemma_set_empty_equivalency_len
        forall|a: Set<A>, b: Set<A>|
            (a.finite() && b.finite() && a.disjoint(b)) ==> #[trigger] (a + b).len() == a.len()
                + b.len(),  //from lemma_set_disjoint_lens
        forall|a: Set<A>, b: Set<A>|
            (a.finite() && b.finite()) ==> #[trigger] (a + b).len() + #[trigger] a.intersect(
                b,
            ).len() == a.len() + b.len(),  //from lemma_set_intersect_union_lens
        forall|a: Set<A>, b: Set<A>|
            (a.finite() && b.finite()) ==> ((#[trigger] a.difference(b).len() + b.difference(
                a,
            ).len() + a.intersect(b).len() == (a + b).len()) && (a.difference(b).len() == a.len()
                - a.intersect(b).len())),  //from lemma_set_difference_len
{
    broadcast use group_set_properties;

    assert forall|s: Set<A>| #[trigger] s.len() != 0 && s.finite() implies exists|a: A|
        s.contains(a) by {
        assert(s.contains(s.choose()));
    }
}

pub broadcast group group_set_properties {
    lemma_set_union_again1,
    lemma_set_union_again2,
    lemma_set_intersect_again1,
    lemma_set_intersect_again2,
    lemma_set_difference2,
    lemma_set_disjoint,
    lemma_set_disjoint_lens,
    lemma_set_intersect_union_lens,
    lemma_set_difference_len,
    // REVIEW: exclude from broadcast group if trigger is too free
    //         also note that some proofs in seq_lib requires this lemma
    lemma_set_empty_equivalency_len,
}

pub broadcast proof fn axiom_is_empty<A>(s: Set<A>)
    requires
        !(#[trigger] s.is_empty()),
    ensures
        exists|a: A| s.contains(a),
{
    admit();  // REVIEW, should this be in `set`, or have a proof?
}

pub broadcast proof fn axiom_is_empty_len0<A>(s: Set<A>)
    ensures
        #[trigger] s.is_empty() <==> (s.finite() && s.len() == 0),
{
}

#[doc(hidden)]
#[verifier::inline]
pub open spec fn check_argument_is_set<A>(s: Set<A>) -> Set<A> {
    s
}

/// Prove two sets equal by extensionality. Usage is:
///
/// ```rust
/// assert_sets_equal!(set1 == set2);
/// ```
///
/// or,
///
/// ```rust
/// assert_sets_equal!(set1 == set2, elem => {
///     // prove that set1.contains(elem) iff set2.contains(elem)
/// });
/// ```
#[macro_export]
macro_rules! assert_sets_equal {
    [$($tail:tt)*] => {
        $crate::vstd::prelude::verus_proof_macro_exprs!($crate::vstd::set_lib::assert_sets_equal_internal!($($tail)*))
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! assert_sets_equal_internal {
    (::vstd::prelude::spec_eq($s1:expr, $s2:expr)) => {
        $crate::vstd::set_lib::assert_sets_equal_internal!($s1, $s2)
    };
    (::vstd::prelude::spec_eq($s1:expr, $s2:expr), $elem:ident $( : $t:ty )? => $bblock:block) => {
        $crate::vstd::set_lib::assert_sets_equal_internal!($s1, $s2, $elem $( : $t )? => $bblock)
    };
    (crate::prelude::spec_eq($s1:expr, $s2:expr)) => {
        $crate::vstd::set_lib::assert_sets_equal_internal!($s1, $s2)
    };
    (crate::prelude::spec_eq($s1:expr, $s2:expr), $elem:ident $( : $t:ty )? => $bblock:block) => {
        $crate::vstd::set_lib::assert_sets_equal_internal!($s1, $s2, $elem $( : $t )? => $bblock)
    };
    (crate::verus_builtin::spec_eq($s1:expr, $s2:expr)) => {
        $crate::vstd::set_lib::assert_sets_equal_internal!($s1, $s2)
    };
    (crate::verus_builtin::spec_eq($s1:expr, $s2:expr), $elem:ident $( : $t:ty )? => $bblock:block) => {
        $crate::vstd::set_lib::assert_sets_equal_internal!($s1, $s2, $elem $( : $t )? => $bblock)
    };
    ($s1:expr, $s2:expr $(,)?) => {
        $crate::vstd::set_lib::assert_sets_equal_internal!($s1, $s2, elem => { })
    };
    ($s1:expr, $s2:expr, $elem:ident $( : $t:ty )? => $bblock:block) => {
        #[verifier::spec] let s1 = $crate::vstd::set_lib::check_argument_is_set($s1);
        #[verifier::spec] let s2 = $crate::vstd::set_lib::check_argument_is_set($s2);
        $crate::vstd::prelude::assert_by($crate::vstd::prelude::equal(s1, s2), {
            $crate::vstd::prelude::assert_forall_by(|$elem $( : $t )?| {
                $crate::vstd::prelude::ensures(
                    $crate::vstd::prelude::imply(s1.contains($elem), s2.contains($elem))
                    &&
                    $crate::vstd::prelude::imply(s2.contains($elem), s1.contains($elem))
                );
                { $bblock }
            });
            $crate::vstd::prelude::assert_($crate::vstd::prelude::ext_equal(s1, s2));
        });
    }
}

pub broadcast group group_set_lib_default {
    axiom_is_empty,
    axiom_is_empty_len0,
    lemma_set_subset_finite,
}

pub use assert_sets_equal_internal;
pub use assert_sets_equal;

} // verus!
