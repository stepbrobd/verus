#![allow(unused_imports)]

// ANCHOR: full
use verus_builtin::*;
use verus_builtin_macros::*;
use verus_state_machines_macros::tokenized_state_machine;
use vstd::cell;
use vstd::cell::*;
use vstd::invariant::*;
use vstd::multiset::*;
use vstd::pervasive::*;
use vstd::prelude::*;
use vstd::shared::*;

verus! {

//////////////////////////////////////////////////////////////////////////////
pub enum BorrowFlag {
    MutBorrow,
    ReadBorrow(nat),  // 0 if there are no borrows
}

type Perm<S> = cell::PointsTo<S>;

// ANCHOR: fields
tokenized_state_machine!(RefCounter<S> {
    fields {
        #[sharding(constant)]
        pub pcell_loc: CellId,

        #[sharding(variable)]
        pub flag: BorrowFlag,

        #[sharding(storage_option)]
        pub storage: Option<Perm<S>>,

        #[sharding(multiset)]
        pub reader: Multiset<Perm<S>>,

        #[sharding(bool)]
        pub writer: bool,
    }
// ANCHOR_END: fields

    #[invariant]
    pub fn reader_agrees_storage(&self) -> bool {
        forall |t: Perm<S>| #[trigger] self.reader.count(t) > 0 ==>
            self.storage == Option::Some(t)
    }

    #[invariant]
    pub fn flag_inv(&self) -> bool {
        match self.flag {
            BorrowFlag::MutBorrow => {
                self.writer && self.reader == Multiset::<Perm<S>>::empty()
                  && self.storage is None
            }
            BorrowFlag::ReadBorrow(n) => {
                !self.writer
                  && self.storage is Some
                  && self.reader.count(self.storage->0) == n
            }
        }
    }

    #[invariant]
    pub fn storage_inv(&self) -> bool {
        match self.storage {
            Some(x) => x@.pcell == self.pcell_loc && x.is_init(),
            None => true,
        }
    }

    init!{
        initialize_empty(loc: CellId) {
            init pcell_loc = loc;
            init flag = BorrowFlag::MutBorrow;
            init storage = Option::None;
            init reader = Multiset::empty();
            init writer = true;
        }
    }

    #[inductive(initialize_empty)]
    fn initialize_empty_inductive(post: Self, loc: CellId) { }

    transition!{
        do_deposit(x: Perm<S>) {
            require(x@.pcell == pre.pcell_loc && x.is_init());
            remove writer -= true;
            assert(pre.flag == BorrowFlag::MutBorrow);
            update flag = BorrowFlag::ReadBorrow(0);

            deposit storage += Some(x);
        }
    }

    #[inductive(do_deposit)]
    fn do_deposit_inductive(pre: Self, post: Self, x: Perm<S>) { }

    transition!{
        do_withdraw() {
            require(pre.flag == BorrowFlag::ReadBorrow(0));
            update flag = BorrowFlag::MutBorrow;

            add writer += true;

            withdraw storage -= Some(let x);
            assert(x@.pcell == pre.pcell_loc && x.is_init());
        }
    }

    #[inductive(do_withdraw)]
    fn do_withdraw_inductive(pre: Self, post: Self) {
        assert_multisets_equal!(post.reader, Multiset::<Perm<S>>::empty());
    }

    property!{
        reader_guard(x: Perm<S>) {
            have reader >= {x};
            guard storage >= Some(x);
        }
    }

    transition!{
        new_reader() {
            require let BorrowFlag::ReadBorrow(n) = pre.flag;
            update flag = BorrowFlag::ReadBorrow(n + 1);

            birds_eye let x = pre.storage->0;
            add reader += { x };
            assert(x@.pcell == pre.pcell_loc && x.is_init());
        }
    }

    #[inductive(new_reader)]
    fn new_reader_inductive(pre: Self, post: Self) { }

    transition!{
        drop_reader(x: Perm<S>) {
            remove reader -= { x };
            assert let BorrowFlag::ReadBorrow(n) = pre.flag;
            assert n >= 1;
            update flag = BorrowFlag::ReadBorrow((n - 1) as nat);
        }
    }

    #[inductive(drop_reader)]
    fn drop_reader_inductive(pre: Self, post: Self, x: Perm<S>) {
        assert(pre.reader.count(x) > 0);
        assert(pre.storage == Option::Some(x));
        assert(pre.storage is Some);
    }
});

pub tracked struct GhostStuff<S> {
    tracked rc_perm: cell::PointsTo<isize>,
    tracked flag_token: RefCounter::flag<S>,
}

impl<S> GhostStuff<S> {
    pub closed spec fn wf(self, inst: RefCounter::Instance<S>, rc_cell: PCell<isize>) -> bool {
        &&& self.rc_perm@.pcell == rc_cell.id()
        &&& self.flag_token.instance_id() == inst.id()
        &&& self.rc_perm.is_init()
        &&& self.rc_perm.value() as int == match self.flag_token.value() {
            BorrowFlag::MutBorrow => 1,
            BorrowFlag::ReadBorrow(n) => -n,
        }
    }
}

struct_with_invariants!{
    pub struct RefCell<S> {
        // 0: no reference taken
        // 1: mut reference taken
        // -n: n non-mut references taken
        rc_cell: PCell<isize>,
        value_cell: PCell<S>,

        inst: Tracked< RefCounter::Instance<S> >,
        inv: Tracked< Shared<LocalInvariant<_, GhostStuff<S>, _>> >,
    }

    pub closed spec fn wf(self) -> bool {
        predicate {
            &&& self.inst@.pcell_loc() == self.value_cell.id()
        }

        invariant on inv with (inst, rc_cell)
            specifically (self.inv@@)
            is (v: GhostStuff<S>)
        {
            v.wf(inst@, rc_cell)
        }
    }
}

pub struct Ref<'a, S> {
    ref_cell: &'a RefCell<S>,
    reader: Tracked<RefCounter::reader<S>>,
}

impl<'a, S> Ref<'a, S> {
    pub closed spec fn wf(&self) -> bool {
        self.ref_cell.wf()
            && self.reader@.instance_id() == self.ref_cell.inst@.id()
            && self.reader@.element()@.pcell == self.ref_cell.value_cell.id()
            && self.reader@.element().is_init()
    }

    pub closed spec fn value(&self) -> S {
        self.reader@.element().value()
    }
}

pub struct RefMut<'a, S> {
    ref_cell: &'a RefCell<S>,
    writer: Tracked<RefCounter::writer<S>>,
    perm: Tracked<Perm<S>>,
}

impl<'a, S> RefMut<'a, S> {
    pub closed spec fn wf(&self) -> bool {
        self.ref_cell.wf()
          && self.writer@.instance_id() == self.ref_cell.inst@.id()
          && self.perm@@.pcell == self.ref_cell.value_cell.id()
          && self.perm@.is_init()
    }

    pub closed spec fn value(&self) -> S {
        self.perm@.value()
    }
}

impl<S> RefCell<S> {
    fn new(s: S) -> (ref_cell: Self)
        ensures
            ref_cell.wf(),
    {
        let (rc_cell, Tracked(rc_perm)) = PCell::new(0);
        let (value_cell, Tracked(value_perm)) = PCell::new(s);
        let tracked (Tracked(inst), Tracked(flag), _, Tracked(writer)) = RefCounter::Instance::<
            S,
        >::initialize_empty(value_cell.id(), None);
        proof {
            inst.do_deposit(value_perm, &mut flag, value_perm, writer.tracked_unwrap());
        }
        let tracked_inst = Tracked(inst);
        let tracked inv = LocalInvariant::new(
            (tracked_inst, rc_cell),
            GhostStuff { rc_perm, flag_token: flag },
            0,
        );
        RefCell::<S> { rc_cell, value_cell, inst: tracked_inst, inv: Tracked(Shared::new(inv)) }
    }

    fn try_borrow<'a>(&'a self) -> (opt_ref: Option<Ref<'a, S>>)
        requires
            self.wf(),
        ensures
            match opt_ref {
                Some(read_ref) => read_ref.wf(),
                None => true,
            },
    {
        let return_value;
        open_local_invariant!(self.inv.borrow().borrow() => g => {
            let tracked GhostStuff { rc_perm: mut rc_perm, flag_token: mut flag_token } = g;

            let cur_rc = *self.rc_cell.borrow(Tracked(&rc_perm));

            if cur_rc <= 0 && cur_rc > isize::MIN {
                let new_rc = cur_rc - 1;
                self.rc_cell.write(Tracked(&mut rc_perm), new_rc);

                let tracked (_, Tracked(reader_token)) =
                    self.inst.borrow().new_reader(&mut flag_token);
                return_value = Some(Ref {
                    ref_cell: self,
                    reader: Tracked(reader_token),
                });
            } else {
                return_value = None;
            }

            proof { g = GhostStuff { rc_perm, flag_token }; }
        });
        return_value
    }

    fn try_borrow_mut<'a>(&'a self) -> (opt_ref_mut: Option<RefMut<'a, S>>)
        requires
            self.wf(),
        ensures
            match opt_ref_mut {
                Some(write_ref) => write_ref.wf(),
                None => true,
            },
    {
        let return_value;
        open_local_invariant!(self.inv.borrow().borrow() => g => {
            let tracked GhostStuff { rc_perm: mut rc_perm, flag_token: mut flag_token } = g;

            let cur_rc = *self.rc_cell.borrow(Tracked(&rc_perm));

            if cur_rc == 0 {
                let new_rc = 1;
                self.rc_cell.write(Tracked(&mut rc_perm), new_rc);

                let tracked (Tracked(perm), Tracked(writer_token)) =
                    self.inst.borrow().do_withdraw(&mut flag_token);
                return_value = Some(RefMut {
                    ref_cell: self,
                    writer: Tracked(writer_token),
                    perm: Tracked(perm),
                });
            } else {
                return_value = None;
            }

            proof { g = GhostStuff { rc_perm, flag_token }; }
        });
        return_value
    }
}

impl<'a, S> Ref<'a, S> {
    fn borrow<'b>(&'b self) -> (s: &'b S)
        requires
            self.wf(),
        ensures
            *s == self.value(),
    {
        self.ref_cell.value_cell.borrow(
            Tracked(
                self.ref_cell.inst.borrow().reader_guard(self.reader@.element(), self.reader.borrow()),
            ),
        )
    }

    fn dispose(self)
        requires
            self.wf(),
    {
        let Ref { ref_cell, reader: Tracked(reader) } = self;
        open_local_invariant!(ref_cell.inv.borrow().borrow() => g => {
            let tracked GhostStuff { rc_perm: mut rc_perm, flag_token: mut flag_token } = g;

            proof {
                ref_cell.inst.borrow().drop_reader(reader.element(), &mut flag_token, reader);
            }

            let cur_rc = *ref_cell.rc_cell.borrow(Tracked(&rc_perm));
            let new_rc = cur_rc + 1;
            ref_cell.rc_cell.write(Tracked(&mut rc_perm), new_rc);

            proof { g = GhostStuff { rc_perm, flag_token }; }
        });
    }
}

impl<'a, S> RefMut<'a, S> {
    fn replace(&mut self, in_s: S) -> (out_s: S)
        requires
            old(self).wf(),
        ensures
            self.wf(),
            out_s == old(self).value(),
            in_s == self.value(),
    {
        self.ref_cell.value_cell.replace(Tracked(self.perm.borrow_mut()), in_s)
    }

    fn dispose(self)
        requires
            self.wf(),
    {
        let RefMut { ref_cell, writer: Tracked(writer), perm: Tracked(perm) } = self;
        open_local_invariant!(ref_cell.inv.borrow().borrow() => g => {
            let tracked GhostStuff { rc_perm: mut rc_perm, flag_token: mut flag_token } = g;

            proof {
                ref_cell.inst.borrow().do_deposit(perm, &mut flag_token, perm, writer);
            }

            let new_rc = 0;
            ref_cell.rc_cell.write(Tracked(&mut rc_perm), new_rc);

            proof { g = GhostStuff { rc_perm, flag_token }; }
        });
    }
}

fn main() {
    let rf = RefCell::new(5);
    let read_ref1 = match rf.try_borrow() {
        Some(x) => x,
        None => {
            return ;
        },
    };
    let read_ref2 = match rf.try_borrow() {
        Some(x) => x,
        None => {
            return ;
        },
    };
    let x = *read_ref1.borrow();
    let y = *read_ref2.borrow();
    print_u64(x);
    print_u64(y);
    read_ref1.dispose();
    read_ref2.dispose();
    let mut write_ref = match rf.try_borrow_mut() {
        Some(x) => x,
        None => {
            return ;
        },
    };
    let t = write_ref.replace(20);
    print_u64(t);
    write_ref.dispose();
    let read_ref3 = match rf.try_borrow() {
        Some(x) => x,
        None => {
            return ;
        },
    };
    let z = *read_ref3.borrow();
    print_u64(z);
    read_ref3.dispose();
}

} // verus!
