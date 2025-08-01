use crate::attributes::{GhostBlockAttr, get_ghost_block_opt, get_mode, get_verifier_attrs};
use crate::erase::{ErasureHints, ResolvedCall};
use crate::external::CrateItems;
use crate::resolve_traits::{ResolutionResult, ResolvedItem};
use crate::rust_to_vir_base::{
    auto_deref_supported_for_ty, def_id_to_vir_path, mid_ty_const_to_vir,
};
use crate::rust_to_vir_ctor::{AdtKind, resolve_braces_ctor, resolve_ctor};
use crate::verus_items::{BuiltinTypeItem, ExternalItem, RustItem, VerusItem, VerusItems};
use crate::{lifetime_ast::*, verus_items};
use air::ast_util::str_ident;
use rustc_ast::{BindingMode, BorrowKind, IsAuto, Mutability};
use rustc_hir::def::{CtorKind, DefKind, Res};
use rustc_hir::{
    AssocItemKind, BinOpKind, Block, BlockCheckMode, BodyId, Closure, Crate, Expr, ExprKind, FnSig,
    HirId, Impl, ImplItem, ImplItemKind, ItemKind, LetExpr, LetStmt, MaybeOwner, Node, OwnerNode,
    Pat, PatExpr, PatExprKind, PatKind, Safety, Stmt, StmtKind, TraitFn, TraitItem, TraitItemKind,
    TraitItemRef, UnOp,
};
use rustc_middle::ty::{
    AdtDef, BoundRegionKind, BoundVariableKind, ClauseKind, Const, GenericArgKind,
    GenericParamDefKind, RegionKind, TermKind, Ty, TyCtxt, TyKind, TypeckResults, TypingEnv,
    VariantDef,
};
use rustc_span::Span;
use rustc_span::def_id::DefId;
use rustc_span::symbol::kw;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use vir::ast::{AutospecUsage, DatatypeTransparency, Dt, Fun, FunX, Function, Mode, Path};
use vir::ast_util::get_field;
use vir::def::{VERUS_SPEC, field_ident_from_rust};
use vir::messages::AstId;

impl TypX {
    fn mk_unit() -> Typ {
        Box::new(TypX::Tuple(vec![]))
    }

    fn mk_bool() -> Typ {
        Box::new(TypX::Primitive("bool".to_string()))
    }

    fn as_lifetime(self) -> Id {
        match self {
            TypX::TypParam(x) => x,
            _ => panic!("expected lifetime param"),
        }
    }
}

struct Context<'tcx> {
    _cmd_line_args: crate::config::Args,
    tcx: TyCtxt<'tcx>,
    verus_items: Arc<VerusItems>,
    types_opt: Option<&'tcx TypeckResults<'tcx>>,
    /// Map each function path to its VIR Function, or to None if it is a #[verifier(external)]
    /// function
    functions: HashMap<Fun, Option<Function>>,
    /// Map each datatype path to its VIR Datatype
    datatypes: HashMap<Path, vir::ast::Datatype>,
    ignored_functions: HashSet<DefId>,
    calls: HashMap<HirId, ResolvedCall>,
    /// Mode of each if/else or match condition, used to decide how to erase if/else and match
    /// condition.  For example, in "if x < 10 { x + 1 } else { x + 2 }", this will record the span
    /// and mode of the expression "x < 10"
    condition_modes: HashMap<HirId, Mode>,
    /// Mode of each variable declaration and use.  For example, in
    /// "if x < 10 { x + 1 } else { x + 2 }", we will have three entries, one for each
    /// occurence of "x"
    var_modes: HashMap<HirId, Mode>,
    ret_spec: Option<bool>,
}

impl<'tcx> Context<'tcx> {
    fn types(&self) -> &'tcx TypeckResults<'tcx> {
        self.types_opt.expect("Context.types")
    }
}

struct ConstOrStaticImport {
    id: DefId,
    is_static: bool,
}

pub(crate) struct State {
    rename_count: usize,
    reached: HashSet<(Option<Path>, DefId)>,
    const_static_worklist: Vec<ConstOrStaticImport>,
    datatype_worklist: Vec<DefId>,
    impl_assocs_worklist: Vec<DefId>,
    imported_fun_worklist: Vec<DefId>,
    id_to_name: HashMap<(String, usize), Id>,
    field_to_name: HashMap<String, Id>,
    typ_param_to_name: HashMap<(String, Option<u32>), Id>,
    lifetime_to_name: HashMap<(String, Option<u32>), Id>,
    fun_to_name: HashMap<Fun, Id>,
    trait_to_name: HashMap<Path, Id>,
    datatype_to_name: HashMap<Path, Id>,
    variant_to_name: HashMap<String, Id>,
    unmangle_names: HashMap<String, String>,
    pub(crate) trait_decl_set: HashSet<Path>,
    pub(crate) trait_decls: Vec<TraitDecl>,
    pub(crate) datatype_decls: Vec<DatatypeDecl>,
    pub(crate) trait_impls: Vec<TraitImpl>,
    pub(crate) fun_decls: Vec<FunDecl>,
    // For an impl "bounds ==> trait T(...t...)", point some or all of t to impl:
    // (We add each impl for T to this when we process T)
    // (To avoid importing unnecessary impls, we delay processing impl until all t are used)
    // t1 -> impl, ..., tn -> impl
    typs_used_in_trait_impls_reverse_map: HashMap<DefId, Vec<DefId>>,
    // impl -> (t1, ..., tn) and process impl when t1...tn is empty
    remaining_typs_needed_for_each_impl: HashMap<DefId, (Id, Vec<DefId>)>,
    enclosing_fun_id: Option<DefId>,
    enclosing_trait_ids: Vec<DefId>,
    inside_trait_decl: u32,
    // inner for<'a> can conflict with outer fn f<'a>, so rename the inner 'a
    rename_bound_for: Vec<Id>,
}

impl State {
    fn new() -> State {
        State {
            rename_count: 0,
            reached: HashSet::new(),
            const_static_worklist: Vec::new(),
            datatype_worklist: Vec::new(),
            impl_assocs_worklist: Vec::new(),
            imported_fun_worklist: Vec::new(),
            id_to_name: HashMap::new(),
            field_to_name: HashMap::new(),
            typ_param_to_name: HashMap::new(),
            lifetime_to_name: HashMap::new(),
            fun_to_name: HashMap::new(),
            trait_to_name: HashMap::new(),
            datatype_to_name: HashMap::new(),
            variant_to_name: HashMap::new(),
            unmangle_names: HashMap::new(),
            trait_decl_set: HashSet::new(),
            trait_decls: Vec::new(),
            datatype_decls: Vec::new(),
            trait_impls: Vec::new(),
            fun_decls: Vec::new(),
            typs_used_in_trait_impls_reverse_map: HashMap::new(),
            remaining_typs_needed_for_each_impl: HashMap::new(),
            enclosing_fun_id: None,
            enclosing_trait_ids: Vec::new(),
            inside_trait_decl: 0,
            rename_bound_for: Vec::new(),
        }
    }

    fn id_with_unmangle<Key: Clone + Eq + std::hash::Hash>(
        rename_count: &mut usize,
        key_to_name: &mut HashMap<Key, Id>,
        unmangle_names: Option<&mut HashMap<String, String>>,
        kind: IdKind,
        key: &Key,
        mk_raw_id: impl Fn() -> String,
    ) -> Id {
        let name = key_to_name.get(key);
        if let Some(name) = name {
            return name.clone();
        }
        *rename_count += 1;
        let raw_id = mk_raw_id();
        let name = Id::new(kind, *rename_count, raw_id.clone());
        key_to_name.insert(key.clone(), name.clone());
        if let Some(unmangle_names) = unmangle_names {
            unmangle_names.insert(name.to_string(), raw_id);
        }
        name
    }

    fn id<Key: Clone + Eq + std::hash::Hash>(
        rename_count: &mut usize,
        key_to_name: &mut HashMap<Key, Id>,
        kind: IdKind,
        key: &Key,
        mk_raw_id: impl Fn() -> String,
    ) -> Id {
        Self::id_with_unmangle(rename_count, key_to_name, None, kind, key, mk_raw_id)
    }

    fn local<S: Into<String>>(&mut self, raw_id: S, local_id_index: usize) -> Id {
        let raw_id = raw_id.into();
        let f = || raw_id.clone();
        let key = (raw_id.to_string(), local_id_index);
        let name = Self::id_with_unmangle(
            &mut self.rename_count,
            &mut self.id_to_name,
            Some(&mut self.unmangle_names),
            IdKind::Local,
            &key,
            f,
        );
        name
    }

    fn field<S: Into<String>>(&mut self, raw_id: S) -> Id {
        let raw_id = raw_id.into();
        let f = || raw_id.clone();
        Self::id(&mut self.rename_count, &mut self.field_to_name, IdKind::Field, &raw_id, f)
    }

    pub(crate) fn typ_param<S: Into<String>>(
        &mut self,
        raw_id: S,
        maybe_impl_index: Option<u32>,
    ) -> Id {
        let raw_id = raw_id.into();
        let (is_impl, impl_index) = match (raw_id.starts_with("impl "), maybe_impl_index) {
            (false, _) => (false, None),
            (true, None) => panic!("unexpected impl type"),
            (true, Some(i)) => (true, Some(i)),
        };
        let f = || if is_impl { "impl".to_string() } else { raw_id.clone() };
        let key = (raw_id.clone(), impl_index);
        Self::id(&mut self.rename_count, &mut self.typ_param_to_name, IdKind::TypParam, &key, f)
    }

    fn lifetime(&mut self, key: (String, Option<u32>)) -> Id {
        let (raw_id, _maybe_disambiguator) = &key;
        let f = || raw_id.replace("'", "");
        let mut id = Self::id(
            &mut self.rename_count,
            &mut self.lifetime_to_name,
            IdKind::Lifetime(false),
            &key,
            f,
        );
        if self.rename_bound_for.contains(&id) {
            id.kind = IdKind::Lifetime(true);
        }
        id
    }

    fn fun_name<'tcx>(&mut self, fun: &Fun) -> Id {
        let f = || fun.path.segments.last().expect("path").to_string();
        Self::id(&mut self.rename_count, &mut self.fun_to_name, IdKind::Fun, fun, f)
    }

    pub(crate) fn trait_name<'tcx>(&mut self, path: &Path) -> Id {
        let f = || path.segments.last().expect("path").to_string();
        Self::id(&mut self.rename_count, &mut self.trait_to_name, IdKind::Trait, path, f)
    }

    pub(crate) fn datatype_name<'tcx>(&mut self, path: &Path) -> Id {
        let f = || path.segments.last().expect("path").to_string();
        Self::id_with_unmangle(
            &mut self.rename_count,
            &mut self.datatype_to_name,
            Some(&mut self.unmangle_names),
            IdKind::Datatype,
            path,
            f,
        )
    }

    fn variant<S: Into<String>>(&mut self, raw_id: S) -> Id {
        let raw_id = raw_id.into();
        let f = || raw_id.clone();
        Self::id(&mut self.rename_count, &mut self.variant_to_name, IdKind::Variant, &raw_id, f)
    }

    pub(crate) fn restart_names(&mut self) {
        self.id_to_name.clear();
        self.field_to_name.clear();
        self.typ_param_to_name.clear();
        self.lifetime_to_name.clear();
        self.fun_to_name.clear();
        self.trait_to_name.clear();
        self.datatype_to_name.clear();
        self.variant_to_name.clear();
    }

    pub(crate) fn unmangle_names<S: Into<String>>(&self, s: S) -> String {
        let mut s = s.into();
        for (name, raw_id) in &self.unmangle_names {
            if s.contains(name) {
                s = s.replace(name, raw_id);
            }
        }
        s
    }

    fn reach_const_static(&mut self, id: DefId, is_static: bool) {
        if id.as_local().is_none() && !self.reached.contains(&(None, id)) {
            self.reached.insert((None, id));
            self.const_static_worklist.push(ConstOrStaticImport { id, is_static });
        }
    }

    fn reach_datatype(&mut self, ctxt: &Context, id: DefId) {
        if !self.reached.contains(&(None, id)) {
            if !matches!(ctxt.verus_items.id_to_name.get(&id), Some(VerusItem::BuiltinType(_))) {
                self.reached.insert((None, id));
                self.datatype_worklist.push(id);
            }
            if let Some(impl_ids) = self.typs_used_in_trait_impls_reverse_map.remove(&id) {
                // Wake up any impls waiting for our type to be reached
                for impl_id in impl_ids {
                    if let Some((_, ref mut ts)) =
                        self.remaining_typs_needed_for_each_impl.get_mut(&impl_id)
                    {
                        // Remove ourself from what impl_id is waiting on
                        ts.retain(|t| *t != id);
                    }
                    self.reach_impl_assoc(impl_id);
                }
            }
        }
    }

    fn reach_impl_assoc(&mut self, id: DefId) {
        if !self.remaining_typs_needed_for_each_impl.contains_key(&id) {
            // Haven't reached trait, or already finished
            return;
        }
        if self.remaining_typs_needed_for_each_impl[&id].1.len() > 0 {
            // We haven't reached all the types we would need to justify this impl
            return;
        }

        if !self.reached.contains(&(None, id)) {
            self.reached.insert((None, id));
            self.impl_assocs_worklist.push(id);
        }
    }

    fn reach_fun(&mut self, id: DefId) {
        if id.as_local().is_none() && !self.reached.contains(&(None, id)) {
            self.reached.insert((None, id));
            self.imported_fun_worklist.push(id);
        }
    }
}

fn span_dummy() -> Span {
    let lo = rustc_span::BytePos(0);
    let hi = rustc_span::BytePos(0);
    let ctxt = rustc_span::SyntaxContext::root();
    let data = rustc_span::SpanData { lo, hi, ctxt, parent: None };
    data.span()
}

fn erase_hir_region<'tcx>(ctxt: &Context<'tcx>, state: &mut State, r: &RegionKind) -> Option<Id> {
    match r {
        RegionKind::ReEarlyParam(bound) => {
            Some(state.lifetime((bound.name.to_string(), Some(bound.index))))
        }
        RegionKind::ReBound(_, bound) => match bound.kind {
            BoundRegionKind::Named(a, _) => Some(state.lifetime(lifetime_key(ctxt, a))),
            _ => None,
        },
        RegionKind::ReStatic => Some(Id::new(IdKind::Builtin, 0, "'static".to_string())),
        RegionKind::ReErased => None,
        _ => {
            dbg!(r);
            panic!("unexpected region")
        }
    }
}

fn erase_generic_const<'tcx>(ctxt: &Context<'tcx>, state: &mut State, cnst: &Const<'tcx>) -> Typ {
    use crate::rustc_middle::query::Key;
    match &*mid_ty_const_to_vir(ctxt.tcx, Some(cnst.default_span(ctxt.tcx)), cnst)
        .expect("mit_ty_const_to_vir failed")
    {
        vir::ast::TypX::TypParam(x) => {
            Box::new(TypX::TypParam(state.typ_param(x.to_string(), None)))
        }
        vir::ast::TypX::ConstInt(i) => Box::new(TypX::Primitive(i.to_string())),
        vir::ast::TypX::ConstBool(b) => Box::new(TypX::Primitive(b.to_string())),
        _ => panic!("GenericArgKind::Const"),
    }
}

fn adt_args<'a, 'tcx>(
    rust_item: Option<RustItem>,
    args: &'a [rustc_middle::ty::GenericArg<'tcx>],
) -> (bool, &'a [rustc_middle::ty::GenericArg<'tcx>]) {
    if rust_item == Some(RustItem::Box)
        || rust_item == Some(RustItem::Rc)
        || rust_item == Some(RustItem::Arc)
        || rust_item == Some(RustItem::AllocGlobal)
        || rust_item == Some(RustItem::ManuallyDrop)
        || rust_item == Some(RustItem::PhantomData)
    {
        (false, args)
    } else {
        (true, args)
    }
}

// Collect some or all as-yet unreached types mentioned directly by ty
// (It's ok to miss some, but the more we capture, the less extraneous code
// we have to import.)
// Return Err if we don't handle an impl with this type.
fn collect_unreached_datatypes<'tcx>(
    ctxt: &Context<'tcx>,
    state: &State,
    datatypes: &mut Vec<DefId>,
    ty: &Ty<'tcx>,
) -> Result<(), ()> {
    match ty.kind() {
        TyKind::Ref(_, t, _) | TyKind::Slice(t) | TyKind::Array(t, _) => {
            collect_unreached_datatypes(ctxt, state, datatypes, t)
        }
        TyKind::Adt(AdtDef(adt_def_data), args) => {
            let did = adt_def_data.did;
            let rust_item = verus_items::get_rust_item(ctxt.tcx, did);
            let (is_user_adt, args) = adt_args(rust_item, args);
            if is_user_adt && !state.reached.contains(&(None, did)) {
                datatypes.push(did);
            }
            for arg in args.iter() {
                match arg.unpack() {
                    rustc_middle::ty::GenericArgKind::Type(t) => {
                        collect_unreached_datatypes(ctxt, state, datatypes, &t)?;
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        TyKind::Bool
        | TyKind::Uint(_)
        | TyKind::Int(_)
        | TyKind::Char
        | TyKind::Str
        | TyKind::Float(_)
        | TyKind::Param(_)
        | TyKind::Never
        | TyKind::Tuple(..)
        | TyKind::RawPtr(..)
        | TyKind::Alias(rustc_middle::ty::AliasTyKind::Projection, _) => Ok(()),
        TyKind::Closure(..) => Err(()),
        TyKind::FnDef(..) => Err(()),
        _ => Err(()),
    }
}

fn erase_ty<'tcx>(ctxt: &Context<'tcx>, state: &mut State, ty: &Ty<'tcx>) -> Typ {
    match ty.kind() {
        TyKind::Bool
        | TyKind::Uint(_)
        | TyKind::Int(_)
        | TyKind::Char
        | TyKind::Str
        | TyKind::Float(_) => Box::new(TypX::Primitive(ty.to_string())),
        TyKind::Param(p) if p.name == kw::SelfUpper => {
            if state.inside_trait_decl > 0 {
                Box::new(TypX::TraitSelf)
            } else {
                Box::new(TypX::TypParam(state.typ_param("Self", None)))
            }
        }
        TyKind::Param(p) => {
            let name = p.name.as_str();
            Box::new(TypX::TypParam(state.typ_param(name.to_string(), Some(p.index))))
        }
        TyKind::Never => Box::new(TypX::Never),
        TyKind::Ref(region, t, mutability) => {
            let lifetime = erase_hir_region(ctxt, state, &region.kind());
            Box::new(TypX::Ref(erase_ty(ctxt, state, t), lifetime, *mutability))
        }
        TyKind::Slice(t) => Box::new(TypX::Slice(erase_ty(ctxt, state, t))),
        TyKind::Array(t, len_const) => {
            let t = erase_ty(ctxt, state, t);
            let t_len = erase_generic_const(ctxt, state, len_const);
            Box::new(TypX::Array(t, t_len))
        }
        TyKind::Tuple(_) => Box::new(TypX::Tuple(
            ty.tuple_fields().iter().map(|t| erase_ty(ctxt, state, &t)).collect(),
        )),
        TyKind::RawPtr(t, mutbl) => {
            let ty = erase_ty(ctxt, state, t);
            Box::new(TypX::RawPtr(ty, *mutbl))
        }
        TyKind::Adt(AdtDef(adt_def_data), args) => {
            let did = adt_def_data.did;
            state.reach_datatype(ctxt, did);

            let path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, did);

            let rust_item = verus_items::get_rust_item(ctxt.tcx, did);
            let (_, args) = adt_args(rust_item, args);

            let (typ_args, _) = erase_generic_args(ctxt, state, args, false);
            let datatype_name = match ctxt.verus_items.id_to_name.get(&did) {
                Some(VerusItem::BuiltinType(t)) => match t {
                    BuiltinTypeItem::Int => Id::new(IdKind::Builtin, 0, "int".to_owned()),
                    BuiltinTypeItem::Nat => Id::new(IdKind::Builtin, 0, "nat".to_owned()),
                    BuiltinTypeItem::FnSpec => Id::new(IdKind::Builtin, 0, "FnSpec".to_owned()),
                    BuiltinTypeItem::Ghost => Id::new(IdKind::Builtin, 0, "Ghost".to_owned()),
                    BuiltinTypeItem::Tracked => Id::new(IdKind::Builtin, 0, "Tracked".to_owned()),
                },
                Some(VerusItem::External(ExternalItem::FnProof)) => {
                    Id::new(IdKind::Builtin, 0, "FnProof".to_owned())
                }
                Some(VerusItem::External(ExternalItem::FOpts)) => {
                    Id::new(IdKind::Builtin, 0, "FOpts".to_owned())
                }
                _ => match rust_item {
                    Some(RustItem::Box) => {
                        assert!(typ_args.len() == 2);
                        Id::new(IdKind::Builtin, 0, "Box".to_owned())
                    }
                    Some(RustItem::Rc) => {
                        assert!(typ_args.len() == 2);
                        Id::new(IdKind::Builtin, 0, "Rc".to_owned())
                    }
                    Some(RustItem::Arc) => {
                        assert!(typ_args.len() == 2);
                        Id::new(IdKind::Builtin, 0, "Arc".to_owned())
                    }
                    Some(RustItem::AllocGlobal) => {
                        assert!(typ_args.len() == 0);
                        Id::new(IdKind::Builtin, 0, "Global".to_owned())
                    }
                    Some(RustItem::ManuallyDrop) => {
                        assert!(typ_args.len() == 1);
                        Id::new(IdKind::Builtin, 0, "ManuallyDrop".to_owned())
                    }
                    Some(RustItem::PhantomData) => {
                        assert!(typ_args.len() == 1);
                        Id::new(IdKind::Builtin, 0, "PhantomData".to_owned())
                    }
                    _ => state.datatype_name(&path),
                },
            };
            Box::new(TypX::Datatype(datatype_name, Vec::new(), typ_args))
        }
        TyKind::Alias(rustc_middle::ty::AliasTyKind::Projection, t) => {
            // Note: even if rust_to_vir_base decides to normalize t,
            // we don't have to normalize t here, since we're generating Rust code, not VIR.
            // However, normalizing means we might reach less stuff so it's
            // still useful.

            // Try normalization:
            use crate::rustc_trait_selection::infer::TyCtxtInferExt;
            use crate::rustc_trait_selection::traits::NormalizeExt;
            if let Some(fun_id) = state.enclosing_fun_id {
                let param_env = ctxt.tcx.param_env(fun_id);
                let ty_mode = rustc_middle::ty::TypingMode::PostAnalysis;
                let infcx = ctxt.tcx.infer_ctxt().ignoring_regions().build(ty_mode);
                let cause = rustc_infer::traits::ObligationCause::dummy();
                let at = infcx.at(&cause, param_env);
                let resolved_ty = infcx.resolve_vars_if_possible(*ty);
                if !rustc_middle::ty::TypeVisitableExt::has_escaping_bound_vars(&resolved_ty) {
                    let norm = at.normalize(*ty);
                    if norm.value != *ty {
                        let mut has_infer = false;
                        for arg in norm.value.walk().into_iter() {
                            if let GenericArgKind::Type(t) = arg.unpack() {
                                if let TyKind::Infer(..) = t.kind() {
                                    // It's not clear why normalize returns Infer
                                    // but it's not what we want
                                    has_infer = true;
                                }
                            }
                        }
                        if !has_infer {
                            return erase_ty(ctxt, state, &norm.value);
                        }
                    }
                }
            }

            // If normalization isn't possible:
            let assoc_item = ctxt.tcx.associated_item(t.def_id);
            let name = state.typ_param(assoc_item.name().to_string(), None);
            let projection_generics = ctxt.tcx.generics_of(t.def_id);
            let trait_def = projection_generics.parent;
            if let Some(trait_def) = trait_def {
                let n = t.args.len() - projection_generics.own_params.len();
                let (trait_typ_args, self_typ) =
                    erase_generic_args(ctxt, state, &t.args[..n], true);

                if Some(trait_def) == ctxt.tcx.lang_items().pointee_trait()
                    && assoc_item.name().as_str() == "Metadata"
                {
                    return Box::new(TypX::PointeeMetadata(self_typ.clone().unwrap()));
                }

                let (assoc_typ_args, _) = erase_generic_args(ctxt, state, &t.args[n..], false);
                let assoc_typ_args = assoc_typ_args.into_iter().map(|a| a.as_lifetime()).collect();
                let self_typ = self_typ.expect("self_typ");
                let trait_path_vir = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, trait_def);
                erase_trait(ctxt, state, trait_def);
                // If the type being erased is in one of the definitions of the trait it references,
                // do not expect it to be in the `trait_decl_set`: we are in the process of erasing
                // this very trait.
                assert!(
                    state.enclosing_trait_ids.contains(&trait_def)
                        || state.trait_decl_set.contains(&trait_path_vir)
                );
                let trait_path = state.trait_name(&trait_path_vir);
                let trait_as_datatype =
                    Box::new(TypX::Datatype(trait_path, Vec::new(), trait_typ_args));
                Box::new(TypX::Projection { self_typ, trait_as_datatype, name, assoc_typ_args })
            } else {
                panic!("unexpected TyKind::Alias");
            }
        }
        TyKind::Closure(..) => Box::new(TypX::Closure),
        TyKind::FnDef(..) => Box::new(TypX::FnDef),
        _ => {
            dbg!(ty);
            panic!("unexpected type")
        }
    }
}

fn erase_generic_args<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    args: &[rustc_middle::ty::GenericArg<'tcx>],
    mut skip_self: bool,
) -> (Vec<Typ>, Option<Typ>) {
    let mut lifetimes: Vec<Typ> = Vec::new();
    let mut typ_args: Vec<Typ> = Vec::new();
    let mut self_typ: Option<Typ> = None;
    for arg in args.iter() {
        match arg.unpack() {
            rustc_middle::ty::GenericArgKind::Type(t) => {
                let typ = erase_ty(ctxt, state, &t);
                if skip_self {
                    self_typ = Some(typ);
                } else {
                    typ_args.push(typ);
                }
                skip_self = false;
            }
            rustc_middle::ty::GenericArgKind::Lifetime(region) => {
                let lifetime = erase_hir_region(ctxt, state, &region.kind());
                let lifetime =
                    lifetime.unwrap_or_else(|| Id::new(IdKind::Builtin, 0, "'_".to_string()));
                lifetimes.push(Box::new(TypX::TypParam(lifetime)));
            }
            rustc_middle::ty::GenericArgKind::Const(cnst) => {
                let t = erase_generic_const(ctxt, state, &cnst);
                typ_args.push(t);
            }
        }
    }
    lifetimes.extend(typ_args);
    (lifetimes, self_typ)
}

fn erase_pat<'tcx>(ctxt: &Context<'tcx>, state: &mut State, pat: &Pat<'tcx>) -> Pattern {
    let mk_pat = |p: PatternX| Box::new((pat.span, p));
    match &pat.kind {
        PatKind::Wild => mk_pat(PatternX::Wildcard),
        PatKind::Expr(PatExpr { kind: PatExprKind::Path(qpath), hir_id, .. }) => {
            let res = ctxt.types().qpath_res(qpath, *hir_id);
            match res {
                Res::Def(DefKind::Const, _id) => mk_pat(PatternX::Wildcard),
                _ => {
                    if let Some((ctor, ctor_kind)) = resolve_ctor(ctxt.tcx, res) {
                        if ctor_kind != CtorKind::Const {
                            panic!("lifetime_generate PatKind::Path: expected CtorKind::Const");
                        }
                        let variant_name = str_ident(&ctor.variant_def.ident(ctxt.tcx).as_str());
                        let vir_path =
                            def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, ctor.adt_def_id);
                        let name = state.datatype_name(&vir_path);
                        let variant = match &ctor.kind {
                            AdtKind::Enum => Some(state.variant(variant_name.to_string())),
                            _ => None,
                        };
                        mk_pat(PatternX::DatatypeTuple(name, variant, vec![], None))
                    } else {
                        panic!("lifetime_generate PatKind::Path: expected ctor");
                    }
                }
            }
        }
        PatKind::Expr(_expr) => mk_pat(PatternX::Wildcard),
        PatKind::Range(_, _, _) => mk_pat(PatternX::Wildcard),
        PatKind::Binding(ann, hir_id, x, None) => {
            if ctxt.var_modes[&pat.hir_id] == Mode::Spec {
                mk_pat(PatternX::Wildcard)
            } else {
                let id = state.local(&x.to_string(), hir_id.local_id.index());
                let BindingMode(_, mutability) = ann;
                mk_pat(PatternX::Binding(id, mutability.to_owned(), None))
            }
        }
        PatKind::Binding(ann, hir_id, x, Some(subpat)) => {
            if ctxt.var_modes[&pat.hir_id] == Mode::Spec {
                erase_pat(ctxt, state, subpat)
            } else {
                let id = state.local(&x.to_string(), hir_id.local_id.index());
                let BindingMode(_, mutability) = ann;
                let subpat = erase_pat(ctxt, state, subpat);
                mk_pat(PatternX::Binding(id, mutability.to_owned(), Some(subpat)))
            }
        }
        PatKind::Box(p) => mk_pat(PatternX::Box(erase_pat(ctxt, state, p))),
        PatKind::Or(pats) => {
            let mut patterns: Vec<Pattern> = Vec::new();
            for pat in pats.iter() {
                patterns.push(erase_pat(ctxt, state, pat));
            }
            mk_pat(PatternX::Or(patterns))
        }
        PatKind::Tuple(pats, dot_dot_pos) => {
            let mut patterns: Vec<Pattern> = Vec::new();
            for pat in pats.iter() {
                patterns.push(erase_pat(ctxt, state, pat));
            }
            mk_pat(PatternX::Tuple(patterns, dot_dot_pos.as_opt_usize()))
        }
        PatKind::TupleStruct(qpath, pats, dot_dot_pos) => {
            let res = ctxt.types().qpath_res(qpath, pat.hir_id);

            if let Some((ctor, ctor_kind)) = resolve_ctor(ctxt.tcx, res) {
                assert!(ctor_kind == CtorKind::Fn);
                let variant_name = str_ident(&ctor.variant_def.ident(ctxt.tcx).as_str());
                let vir_path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, ctor.adt_def_id);

                let name = state.datatype_name(&vir_path);
                let variant_name = state.variant(variant_name.to_string());
                let mut patterns: Vec<Pattern> = Vec::new();
                for pat in pats.iter() {
                    patterns.push(erase_pat(ctxt, state, pat));
                }
                let variant = match &ctor.kind {
                    AdtKind::Enum => Some(variant_name),
                    _ => None,
                };
                mk_pat(PatternX::DatatypeTuple(name, variant, patterns, dot_dot_pos.as_opt_usize()))
            } else {
                panic!("lifetime_generate PatKind::TupleStruct: expected ctor");
            }
        }
        PatKind::Struct(qpath, pats, has_omitted) => {
            let res = ctxt.types().qpath_res(qpath, pat.hir_id);
            let ty = ctxt.types().node_type(pat.hir_id);
            let ctor = resolve_braces_ctor(ctxt.tcx, res, ty, false, pat.span).unwrap();
            let variant_name = str_ident(&ctor.variant_def.ident(ctxt.tcx).as_str());
            let vir_path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, ctor.adt_def_id);

            let variant_opt = match &ctor.kind {
                AdtKind::Enum => Some(state.variant(variant_name.to_string())),
                _ => None,
            };

            let name = state.datatype_name(&vir_path);
            let mut binders: Vec<(Id, Pattern)> = Vec::new();
            for pat in pats.iter() {
                let field = state.field(pat.ident.to_string());
                let pattern = erase_pat(ctxt, state, &pat.pat);
                binders.push((field, pattern));
            }
            mk_pat(PatternX::DatatypeStruct(name, variant_opt, binders, *has_omitted))
        }
        _ => {
            dbg!(pat);
            panic!("unexpected pattern")
        }
    }
}

fn erase_spec_exps_typ<'tcx>(
    _ctxt: &Context,
    state: &mut State,
    span: Span,
    mk_typ: impl FnOnce(&mut State) -> Typ,
    exps: Vec<Option<Exp>>,
    force_some: bool,
) -> Option<Exp> {
    let mk_exp = |e: ExpX| Some(Box::new((span, e)));

    let mut is_some: bool = false;
    let mut args: Vec<Exp> = Vec::new();
    for exp in exps.into_iter() {
        if let Some(exp) = exp {
            args.push(exp);
            is_some = true;
        }
    }
    if is_some || force_some { mk_exp(ExpX::Op(args, mk_typ(state))) } else { None }
}

// Return an Exp instead of an Option<Exp>
// (in particulary, instead of returning None, return a dummy expression with the intended type)
fn erase_spec_exps_force_typ<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    span: Span,
    typ: Typ,
    exps: Vec<Option<Exp>>,
) -> Exp {
    erase_spec_exps_typ(ctxt, state, span, |_| typ, exps, true).expect("erase expr force")
}

fn erase_spec_exps_force<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expr: &Expr<'tcx>,
    exps: Vec<Option<Exp>>,
) -> Exp {
    let expr_typ = |state: &mut State| erase_ty(ctxt, state, &ctxt.types().node_type(expr.hir_id));
    erase_spec_exps_typ(ctxt, state, expr.span, expr_typ, exps, true).expect("erase expr force")
}

fn erase_spec_exps<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expr: &Expr<'tcx>,
    exps: Vec<Option<Exp>>,
) -> Option<Exp> {
    let expr_typ = |state: &mut State| erase_ty(ctxt, state, &ctxt.types().node_type(expr.hir_id));
    erase_spec_exps_typ(ctxt, state, expr.span, expr_typ, exps, false)
}

fn phantom_data_expr<'tcx>(ctxt: &Context<'tcx>, state: &mut State, expr: &Expr<'tcx>) -> Exp {
    let e = erase_expr(ctxt, state, true, expr);
    let ty = ctxt.types().node_type(expr.hir_id);
    let typ = Box::new(TypX::Phantom(erase_ty(ctxt, state, &ty)));
    erase_spec_exps_force_typ(ctxt, state, expr.span, typ, vec![e])
}

// Convert an Option<Exp> into an Exp by converting None into an empty block
// (useful for Rust expressions that require blocks, like if or while)
fn force_block(exp: Option<Exp>, span: Span) -> Exp {
    match exp {
        None => Box::new((span, ExpX::Block(vec![], None))),
        Some(exp @ box (_, ExpX::Block(..))) => exp,
        Some(exp @ box (span, _)) => Box::new((span, ExpX::Block(vec![], Some(exp)))),
    }
}

// Convert an Option<Exp> into an Exp by converting None into a unit value
fn force_exp(exp: Option<Exp>, span: Span) -> Exp {
    match exp {
        None => Box::new((span, ExpX::Tuple(vec![]))),
        Some(e) => e,
    }
}

fn mk_typ_args<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    node_substs: &[rustc_middle::ty::GenericArg<'tcx>],
) -> Vec<Typ> {
    let mut typ_args: Vec<Typ> = Vec::new();
    for typ_arg in node_substs {
        match typ_arg.unpack() {
            GenericArgKind::Type(ty) => {
                typ_args.push(erase_ty(ctxt, state, &ty));
            }
            GenericArgKind::Lifetime(_) => {}
            GenericArgKind::Const(c) => {
                typ_args.push(erase_generic_const(ctxt, state, &c));
            }
        }
    }
    typ_args
}

fn erase_call<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expect_spec: bool,
    expr: &Expr<'tcx>,
    expr_fun: Option<&Expr<'tcx>>,
    fn_def_id: Option<DefId>,
    node_substs: &'tcx rustc_middle::ty::List<rustc_middle::ty::GenericArg<'tcx>>,
    fn_span: Span,
    receiver: Option<&Expr<'tcx>>,
    args_slice: &'tcx [Expr<'tcx>],
    is_method: bool,
    is_variant: bool,
) -> Option<Exp> {
    let mut is_some: bool = false;
    let mk_exp = |e: ExpX| Some(Box::new((expr.span, e)));
    let call = ctxt
        .calls
        .get(&expr.hir_id)
        .unwrap_or_else(|| panic!("internal error: missing function: {:?}", fn_span));
    match call {
        ResolvedCall::Spec => None,
        ResolvedCall::SpecAllowProofArgs => {
            let exps = receiver
                .into_iter()
                .chain(args_slice.iter())
                .map(|a| erase_expr(ctxt, state, expect_spec, a))
                .collect(); // REVIEW(main_new) correct?
            erase_spec_exps_typ(ctxt, state, expr.span, |_| TypX::mk_unit(), exps, false)
        }
        ResolvedCall::CompilableOperator(op) => {
            use crate::erase::CompilableOperator::*;
            let verus_builtin_method = match op {
                SmartPtrClone { is_method } => Some((*is_method, "clone", false)),
                TrackedGet => Some((true, "get", false)),
                TrackedBorrow => Some((true, "borrow", false)),
                TrackedBorrowMut => Some((true, "borrow_mut", false)),
                TrackedNew | TrackedExec => Some((false, "tracked_new", expect_spec)),
                TrackedExecBorrow => Some((false, "tracked_exec_borrow", false)),
                RcNew => Some((false, "rc_new", expect_spec)),
                ArcNew => Some((false, "arc_new", expect_spec)),
                BoxNew => Some((false, "box_new", expect_spec)),
                GhostExec => None,
                IntIntrinsic | Implies => None,
                UseTypeInvariant => Some((false, "use_type_invariant", false)),
                ClosureToFnProof(_) => Some((false, "closure_to_fn_proof", false)),
            };
            if let Some((true, method, expect_spec_inside)) = verus_builtin_method {
                assert!(receiver.is_some());
                assert!(args_slice.len() == 0);
                let Some(receiver) = receiver else { panic!() };
                let exp = erase_expr(ctxt, state, expect_spec_inside, &receiver);
                if expect_spec_inside {
                    erase_spec_exps(ctxt, state, expr, vec![exp])
                } else {
                    mk_exp(ExpX::BuiltinMethod(
                        exp.expect("verus_builtin method"),
                        method.to_string(),
                    ))
                }
            } else if let Some((false, func, expect_spec_inside)) = verus_builtin_method {
                assert!(receiver.is_none());
                assert!(args_slice.len() == 1);
                let exp = if let ClosureToFnProof(mode) = op {
                    Some(erase_expr_closure(ctxt, state, expect_spec_inside, *mode, &args_slice[0]))
                } else {
                    erase_expr(ctxt, state, expect_spec_inside, &args_slice[0])
                };
                if expect_spec_inside {
                    erase_spec_exps(ctxt, state, expr, vec![exp])
                } else {
                    let target =
                        mk_exp(ExpX::Var(Id::new(IdKind::Builtin, 0, func.to_string()))).unwrap();
                    let typ_args = mk_typ_args(ctxt, state, node_substs);
                    mk_exp(ExpX::Call(target, typ_args, vec![exp.expect("verus_builtin method")]))
                }
            } else if let GhostExec = op {
                Some(erase_spec_exps_force(ctxt, state, expr, vec![]))
            } else {
                assert!(receiver.is_none());
                let exps =
                    args_slice.iter().map(|a| erase_expr(ctxt, state, expect_spec, a)).collect();
                erase_spec_exps(ctxt, state, expr, exps)
            }
        }
        ResolvedCall::Call(f_name, autospec_usage) => {
            if !ctxt.functions.contains_key(f_name) {
                panic!("internal error: function call to {:?} not found {:?}", f_name, expr.span);
            }
            let f = &ctxt.functions[f_name];
            let f = if let Some(f) = f {
                f
            } else {
                panic!("internal error: call to external function {:?} {:?}", f_name, expr.span);
            };

            let (f_name, f) = match (autospec_usage, &f.x.attrs.autospec) {
                (AutospecUsage::IfMarked, Some(new_f_name)) => {
                    let f = &ctxt.functions[new_f_name];
                    let f = if let Some(f) = f {
                        f
                    } else {
                        panic!(
                            "internal error: call to external function {:?} {:?}",
                            f_name, expr.span
                        );
                    };
                    (new_f_name.clone(), f.clone())
                }
                _ => (f_name.clone(), f.clone()),
            };

            if f.x.mode == Mode::Spec {
                return None;
            }

            // Maybe resolve from trait function to a specific implementation

            let node_substs = node_substs;
            let mut fn_def_id = fn_def_id.expect("call id");

            let rust_item = crate::verus_items::get_rust_item(ctxt.tcx, fn_def_id);
            let mut node_substs = crate::fn_call_to_vir::fix_node_substs(
                ctxt.tcx,
                ctxt.types(),
                node_substs,
                rust_item,
                &args_slice.iter().collect::<Vec<_>>(),
                expr,
            );

            if ctxt.tcx.trait_of_item(fn_def_id).is_some() {
                let typing_env = TypingEnv::post_analysis(
                    ctxt.tcx,
                    state.enclosing_fun_id.expect("enclosing_fun_id"),
                );
                let resolution_result = crate::resolve_traits::resolve_trait_item(
                    expr.span,
                    ctxt.tcx,
                    typing_env,
                    fn_def_id,
                    node_substs,
                )
                .unwrap();
                match resolution_result {
                    ResolutionResult::Unresolved => {}
                    ResolutionResult::Resolved {
                        resolved_item: ResolvedItem::FromImpl(did, args),
                        ..
                    } => {
                        node_substs = args;
                        fn_def_id = did;
                    }
                    ResolutionResult::Resolved {
                        resolved_item: ResolvedItem::FromTrait(..),
                        ..
                    } => {}
                    ResolutionResult::Builtin(_) => {}
                }
            }

            state.reach_fun(fn_def_id);

            let typ_args = mk_typ_args(ctxt, state, node_substs);
            let mut exps: Vec<Exp> = Vec::new();
            let mut is_first: bool = true;
            assert!(receiver.map(|_| 1).unwrap_or(0) + args_slice.len() == f.x.params.len());
            for (param, e) in f.x.params.iter().zip(receiver.into_iter().chain(args_slice.iter())) {
                if param.x.mode == Mode::Spec {
                    let exp = erase_expr(ctxt, state, true, e);
                    is_some = is_some || exp.is_some();
                    exps.push(erase_spec_exps_force_typ(
                        ctxt,
                        state,
                        e.span,
                        TypX::mk_unit(),
                        vec![exp],
                    ));
                } else {
                    let mut exp = erase_expr(ctxt, state, false, e).expect("expr");
                    if is_first && is_method {
                        let adjustments = ctxt.types().expr_adjustments(e);
                        // There could be more than one adjustments:
                        // For example
                        // 1. mut [u8; N] -> &[u8]: [Borrow(Ref('{erased}, _)) -> &[u8; 10], Pointer(Unsize) -> &[u8]]
                        // 2. Rc<String> -> &str will use two Borrow adjustments
                        for adjust in adjustments {
                            use rustc_middle::ty::adjustment::{
                                Adjust, AutoBorrow, AutoBorrowMutability,
                            };
                            match adjust.kind {
                                Adjust::Borrow(AutoBorrow::Ref(m)) => {
                                    let m = match m {
                                        AutoBorrowMutability::Not => Mutability::Not,
                                        AutoBorrowMutability::Mut { .. } => Mutability::Mut,
                                    };
                                    exp = Box::new((exp.0, ExpX::AddrOf(m, exp)));
                                }
                                Adjust::Deref(None) => {
                                    exp = Box::new((exp.0, ExpX::Deref(exp)));
                                }
                                _ => {}
                            }
                        }
                    }
                    exps.push(exp);
                    is_some = true;
                }
                is_first = false;
            }
            if expect_spec && !is_some {
                None
            } else {
                let name = state.fun_name(&f_name);
                let target = Box::new((fn_span, ExpX::Var(name)));
                mk_exp(ExpX::Call(target, typ_args, exps))
            }
        }
        ResolvedCall::Ctor(path, variant_name) => {
            assert!(receiver.is_none());
            if expect_spec {
                let mut exps: Vec<Option<Exp>> = Vec::new();
                for arg in args_slice.iter() {
                    exps.push(erase_expr(ctxt, state, expect_spec, arg));
                }
                erase_spec_exps(ctxt, state, expr, exps)
            } else {
                let datatype = &ctxt.datatypes[path];
                let variant = datatype.x.get_variant(variant_name);
                let typ_args = mk_typ_args(ctxt, state, node_substs);
                let mut args: Vec<Exp> = Vec::new();
                for (field, arg) in variant.fields.iter().zip(args_slice.iter()) {
                    let (_, field_mode, _) = field.a;
                    let e = if field_mode == Mode::Spec {
                        phantom_data_expr(ctxt, state, arg)
                    } else {
                        erase_expr(ctxt, state, expect_spec, arg).expect("expr")
                    };
                    args.push(e);
                }
                // make sure datatype is generated
                let _ = erase_ty(ctxt, state, &ctxt.types().node_type(expr.hir_id));

                let variant_opt =
                    if is_variant { Some(state.variant(variant_name.to_string())) } else { None };
                mk_exp(ExpX::DatatypeTuple(state.datatype_name(path), variant_opt, typ_args, args))
            }
        }
        ResolvedCall::NonStaticExec | ResolvedCall::NonStaticProof(_) => {
            assert!(receiver.is_none());
            let expr_fun = expr_fun.expect("exec closure call function target");
            let exp_fun = erase_expr(ctxt, state, false, expr_fun).expect("closure call target");
            let typ_args = mk_typ_args(ctxt, state, node_substs);
            let mut exps: Vec<Exp> = Vec::new();
            let modes = if let ResolvedCall::NonStaticProof(modes) = &call {
                modes.clone()
            } else {
                Arc::new(args_slice.iter().map(|_| Mode::Exec).collect())
            };
            assert!(args_slice.len() == modes.len());
            for (a, mode) in args_slice.iter().zip(modes.iter()) {
                if *mode == Mode::Spec {
                    let spec_exp = erase_expr(ctxt, state, true, a);
                    let ty = ctxt.types().node_type(a.hir_id);
                    let typ = erase_ty(ctxt, state, &ty);
                    exps.push(erase_spec_exps_force_typ(ctxt, state, a.span, typ, vec![spec_exp]));
                } else {
                    exps.push(erase_expr(ctxt, state, false, a).expect("call arg"));
                }
            }
            // syntax quirk: need extra parens when exp_fun is a block
            let exp_fun = Box::new((expr_fun.span, ExpX::ExtraParens(exp_fun)));
            mk_exp(ExpX::Call(exp_fun, typ_args, exps))
        }
    }
}

fn erase_match<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expect_spec: bool,
    expr: &Expr<'tcx>,
    cond: &Expr<'tcx>,
    arms: Vec<(Option<&Pat<'tcx>>, &Option<&Expr<'tcx>>, Option<&Expr<'tcx>>)>,
) -> Option<Exp> {
    let expr_typ = |state: &mut State| erase_ty(ctxt, state, &ctxt.types().node_type(expr.hir_id));
    let mk_exp1 = |e: ExpX| Box::new((expr.span, e));
    let mk_exp = |e: ExpX| Some(Box::new((expr.span, e)));
    let mut is_some_arms = false;
    let cond_spec = ctxt.condition_modes[&expr.hir_id] == Mode::Spec;
    let ec = erase_expr(ctxt, state, cond_spec, cond);
    let mut e_arms: Vec<(Pattern, Option<Exp>, Exp)> = Vec::new();
    for (pat_opt, guard_opt, body_expr) in arms.iter() {
        let pattern = if let Some(pat) = pat_opt {
            erase_pat(ctxt, state, pat)
        } else {
            Box::new((span_dummy(), PatternX::Wildcard))
        };
        let guard = match guard_opt {
            None => None,
            Some(guard) => erase_expr(ctxt, state, cond_spec, guard),
        };
        let (body, body_span) = if let Some(b) = body_expr {
            (erase_expr(ctxt, state, expect_spec, b), b.span)
        } else {
            (None, span_dummy())
        };
        is_some_arms = is_some_arms || body.is_some();
        e_arms.push((pattern, guard, force_block(body, body_span)));
    }
    if expect_spec && !is_some_arms {
        erase_spec_exps(ctxt, state, expr, vec![ec])
    } else {
        if expect_spec && e_arms.len() < arms.len() {
            // add default case
            let pattern = Box::new((span_dummy(), PatternX::Wildcard));
            let body = Box::new((span_dummy(), ExpX::Op(vec![], expr_typ(state))));
            e_arms.push((pattern, None, body));
        }
        let c = match ec {
            None => {
                let ctyp = ctxt.types().node_type(cond.hir_id);
                mk_exp1(ExpX::Op(vec![], erase_ty(ctxt, state, &ctyp)))
            }
            Some(e) => e,
        };
        mk_exp(ExpX::Match(c, e_arms))
    }
}

fn erase_inv_block<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    span: Span,
    body: &Block<'tcx>,
) -> Exp {
    assert!(body.stmts.len() == 4);
    let spend_stmt = &body.stmts[0];
    let open_stmt = &body.stmts[1];
    let mid_stmt = &body.stmts[2];
    if !crate::rust_to_vir_expr::is_spend_open_invariant_credit_call(&ctxt.verus_items, spend_stmt)
    {
        panic!("missing spend_open_invariant_credit call for erase_inv_block");
    }
    let (_guard_hir, _inner_hir, inner_pat, arg, atomicity) =
        crate::rust_to_vir_expr::invariant_block_open(&ctxt.verus_items, open_stmt)
            .expect("invariant_block_open");
    let pat_typ = erase_ty(ctxt, state, &ctxt.types().node_type(inner_pat.hir_id));
    let inner_pat = match &inner_pat.kind {
        PatKind::Binding(ann, hir_id, x, None) => {
            let id = state.local(&x.to_string(), hir_id.local_id.index());
            let BindingMode(_, mutability) = ann;
            Box::new((inner_pat.span, PatternX::Binding(id, mutability.to_owned(), None)))
        }
        _ => {
            panic!("unexpected pattern kind for erase_inv_block");
        }
    };
    let arg = erase_expr(ctxt, state, false, arg).expect("erase_inv_block arg");
    let mid_body = erase_stmt(ctxt, state, mid_stmt);
    let mid_exp = Box::new((
        mid_stmt.span,
        ExpX::OpenInvariant(atomicity, inner_pat, arg, pat_typ, mid_body),
    ));
    let spend_body = erase_stmt(ctxt, state, spend_stmt);
    Box::new((span, ExpX::Block(spend_body, Some(mid_exp))))
}

fn erase_expr<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expect_spec: bool,
    expr: &Expr<'tcx>,
) -> Option<Exp> {
    let mut exp = match erase_expr_inner(ctxt, state, expect_spec, expr) {
        None => return None,
        Some(exp) => exp,
    };
    let mut ty = ctxt.types().expr_ty(expr);
    let adjustments = ctxt.types().expr_adjustments(expr);
    let mut has_deref_call = false;
    for adjust in adjustments {
        use rustc_middle::ty::adjustment::{Adjust, AutoBorrow, AutoBorrowMutability};
        match adjust.kind {
            Adjust::Deref(Some(deref)) => {
                if !auto_deref_supported_for_ty(ctxt.tcx, &ty) {
                    // exp := *op<_, &t>(&exp)
                    let typ = erase_ty(ctxt, state, &adjust.target);
                    let typ = Box::new(TypX::Ref(typ, None, deref.mutbl));
                    exp = Box::new((exp.0, ExpX::AddrOf(deref.mutbl, exp)));
                    exp = Box::new((exp.0, ExpX::Op(vec![exp], typ)));
                    exp = Box::new((exp.0, ExpX::Deref(exp)));
                    has_deref_call = true;
                }
            }
            Adjust::Borrow(AutoBorrow::Ref(m)) if has_deref_call => {
                let m = match m {
                    AutoBorrowMutability::Not => Mutability::Not,
                    AutoBorrowMutability::Mut { .. } => Mutability::Mut,
                };
                exp = Box::new((exp.0, ExpX::AddrOf(m, exp)));
            }
            _ => {}
        }
        ty = adjust.target;
    }
    Some(exp)
}

fn erase_expr_inner<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expect_spec: bool,
    expr: &Expr<'tcx>,
) -> Option<Exp> {
    let expr = expr.peel_drop_temps();
    let expr_typ = |state: &mut State| erase_ty(ctxt, state, &ctxt.types().node_type(expr.hir_id));
    let mk_exp1 = |e: ExpX| Box::new((expr.span, e));
    let mk_exp = |e: ExpX| Some(Box::new((expr.span, e)));

    match &expr.kind {
        ExprKind::Path(qpath) => {
            let res = ctxt.types().qpath_res(qpath, expr.hir_id);

            match res {
                Res::Local(id) => match ctxt.tcx.hir_node(id) {
                    Node::Pat(Pat { kind: PatKind::Binding(_ann, id, ident, _pat), .. }) => {
                        if !ctxt.var_modes.contains_key(&expr.hir_id) {
                            dbg!(expr);
                        }
                        if expect_spec || ctxt.var_modes[&expr.hir_id] == Mode::Spec {
                            None
                        } else {
                            mk_exp(ExpX::Var(state.local(&ident.to_string(), id.local_id.index())))
                        }
                    }
                    _ => panic!("unsupported"),
                },
                Res::SelfCtor(_) | Res::Def(DefKind::Ctor(_, _), _) => {
                    if expect_spec {
                        None
                    } else {
                        let (ctor, ctor_kind) = resolve_ctor(ctxt.tcx, res).unwrap();
                        if ctor_kind != CtorKind::Const {
                            panic!("unsupported: this CtorKind here");
                        }
                        let variant_name = str_ident(&ctor.variant_def.ident(ctxt.tcx).as_str());
                        let vir_path =
                            def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, ctor.adt_def_id);

                        let rust_item = verus_items::get_rust_item(ctxt.tcx, ctor.adt_def_id);
                        if rust_item == Some(RustItem::PhantomData) {
                            return mk_exp(ExpX::Var(Id::new(
                                IdKind::Builtin,
                                0,
                                "PhantomData".to_owned(),
                            )));
                        }

                        let variant = if ctor.kind == AdtKind::Enum {
                            Some(state.variant(variant_name.to_string()))
                        } else {
                            None
                        };
                        let typ_args =
                            mk_typ_args(ctxt, state, ctxt.types().node_args(expr.hir_id));
                        return mk_exp(ExpX::DatatypeTuple(
                            state.datatype_name(&vir_path),
                            variant,
                            typ_args,
                            vec![],
                        ));
                    }
                }
                Res::Def(DefKind::AssocConst, id) => {
                    if expect_spec {
                        None
                    } else {
                        state.reach_const_static(id, false);
                        let typ = expr_typ(state);
                        assert!(matches!(*typ, TypX::Primitive(_)));
                        mk_exp(ExpX::Op(vec![], typ))
                    }
                }
                Res::Def(DefKind::Const, id) => {
                    if expect_spec || ctxt.var_modes[&expr.hir_id] == Mode::Spec {
                        None
                    } else {
                        state.reach_const_static(id, false);
                        let vir_path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);
                        let fun_name = Arc::new(FunX { path: vir_path });
                        let fun_exp = mk_exp1(ExpX::Var(state.fun_name(&fun_name)));
                        return mk_exp(ExpX::Call(fun_exp, vec![], vec![]));
                    }
                }
                Res::Def(
                    DefKind::Static { mutability: Mutability::Not, nested: false, .. },
                    id,
                ) => {
                    if expect_spec || ctxt.var_modes[&expr.hir_id] == Mode::Spec {
                        None
                    } else {
                        state.reach_const_static(id, true);
                        let vir_path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);
                        let fun_name = Arc::new(FunX { path: vir_path });
                        let fun_exp = mk_exp1(ExpX::Var(state.fun_name(&fun_name)));
                        let e = mk_exp1(ExpX::Call(fun_exp, vec![], vec![]));
                        // The function we emit to represent the static returns a
                        // &'static reference. So we need to deref here
                        mk_exp(ExpX::Deref(e))
                    }
                }
                Res::Def(DefKind::Fn | DefKind::AssocFn, id) => {
                    if expect_spec || ctxt.var_modes[&expr.hir_id] == Mode::Spec {
                        None
                    } else {
                        state.reach_fun(id);
                        let vir_path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);
                        let fun_name = Arc::new(FunX { path: vir_path });
                        return mk_exp(ExpX::Var(state.fun_name(&fun_name)));
                    }
                }
                Res::Def(DefKind::ConstParam, id) => {
                    let local_id = id.as_local().expect("ConstParam local");
                    let hir_id = ctxt.tcx.local_def_id_to_hir_id(local_id);
                    match ctxt.tcx.hir_node(hir_id) {
                        Node::GenericParam(gp) => {
                            let name = state.typ_param(gp.name.ident().to_string(), None);
                            mk_exp(ExpX::Var(name))
                        }
                        _ => panic!("ConstParam"),
                    }
                }
                _ => {
                    panic!("unsupported")
                }
            }
        }
        ExprKind::Lit(_lit) => {
            if expect_spec {
                None
            } else {
                let typ = expr_typ(state);
                mk_exp(ExpX::Op(vec![], typ))
            }
        }
        ExprKind::Call(e0, es) => {
            let is_variant = match &e0.kind {
                ExprKind::Path(qpath) => {
                    let res = ctxt.types().qpath_res(qpath, e0.hir_id);
                    match res {
                        Res::Def(DefKind::Variant, _did) => true,
                        Res::Def(DefKind::Ctor(rustc_hir::def::CtorOf::Variant, ..), _did) => true,
                        _ => false,
                    }
                }
                _ => false,
            };
            let fn_def_id = if let ExprKind::Path(qpath) = &e0.kind {
                let def = ctxt.types().qpath_res(&qpath, e0.hir_id);
                if let Res::Def(_, fn_def_id) = def { Some(fn_def_id) } else { None }
            } else {
                None
            };
            erase_call(
                ctxt,
                state,
                expect_spec,
                expr,
                Some(e0),
                fn_def_id,
                ctxt.types().node_args(e0.hir_id),
                e0.span,
                None,
                es,
                false,
                is_variant,
            )
        }
        ExprKind::MethodCall(segment, receiver, args, _call_span) => {
            let fn_def_id = ctxt.types().type_dependent_def_id(expr.hir_id).expect("method id");
            erase_call(
                ctxt,
                state,
                expect_spec,
                expr,
                None,
                Some(fn_def_id),
                ctxt.types().node_args(expr.hir_id),
                segment.ident.span,
                Some(receiver),
                args,
                true,
                false,
            )
        }
        ExprKind::Struct(qpath, fields, spread) => {
            if expect_spec {
                let mut exps: Vec<Option<Exp>> = Vec::new();
                for f in fields.iter() {
                    exps.push(erase_expr(ctxt, state, expect_spec, f.expr));
                }
                erase_spec_exps(ctxt, state, expr, exps)
            } else {
                let res = ctxt.types().qpath_res(qpath, expr.hir_id);
                let ty = ctxt.types().node_type(expr.hir_id);

                let ctor = resolve_braces_ctor(ctxt.tcx, res, ty, true, expr.span).unwrap();
                let variant_name = ctor.variant_name(ctxt.tcx, fields);
                let vir_path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, ctor.adt_def_id);

                let datatype = &ctxt.datatypes[&vir_path];
                let variant = datatype.x.get_variant(&variant_name);
                let mut fs: Vec<(Id, Exp)> = Vec::new();
                for f in fields.iter() {
                    let vir_field_name = field_ident_from_rust(f.ident.as_str());
                    let (_, field_mode, _) = get_field(&variant.fields, &vir_field_name).a;
                    let name = state.field(f.ident.to_string());
                    let e = if field_mode == Mode::Spec {
                        phantom_data_expr(ctxt, state, &f.expr)
                    } else {
                        erase_expr(ctxt, state, expect_spec, f.expr).expect("expr")
                    };
                    fs.push((name, e));
                }
                let variant_opt = if ctor.kind == AdtKind::Enum {
                    Some(state.variant(variant_name.to_string()))
                } else {
                    None
                };
                let spread = match spread {
                    rustc_hir::StructTailExpr::None => None,
                    rustc_hir::StructTailExpr::Base(expr) => {
                        Some(erase_expr(ctxt, state, expect_spec, expr).expect("expr"))
                    }
                    rustc_hir::StructTailExpr::DefaultFields(_span) => None,
                };
                let typ_args = if let box TypX::Datatype(_, _, typ_args) = expr_typ(state) {
                    typ_args
                } else {
                    panic!("unexpected struct expression type")
                };
                mk_exp(ExpX::DatatypeStruct(
                    state.datatype_name(&vir_path),
                    variant_opt,
                    typ_args,
                    fs,
                    spread,
                ))
            }
        }
        ExprKind::Tup(exprs) => {
            let mut args: Vec<Exp> = Vec::new();
            if expect_spec {
                let exps = exprs.iter().map(|e| erase_expr(ctxt, state, expect_spec, e)).collect();
                erase_spec_exps(ctxt, state, expr, exps)
            } else {
                for e in exprs.iter() {
                    args.push(erase_expr(ctxt, state, expect_spec, e).expect("expr"));
                }
                mk_exp(ExpX::Tuple(args))
            }
        }
        ExprKind::Array(exprs) => {
            let mut args: Vec<Exp> = Vec::new();
            if expect_spec {
                let exps = exprs.iter().map(|e| erase_expr(ctxt, state, expect_spec, e)).collect();
                erase_spec_exps(ctxt, state, expr, exps)
            } else {
                for e in exprs.iter() {
                    args.push(erase_expr(ctxt, state, expect_spec, e).expect("expr"));
                }
                mk_exp(ExpX::Array(args))
            }
        }
        ExprKind::Repeat(e, _array_len) => {
            let exp = erase_expr(ctxt, state, expect_spec, e);
            if expect_spec {
                erase_spec_exps(ctxt, state, expr, vec![exp])
            } else {
                let array_ty = erase_ty(ctxt, state, &ctxt.types().expr_ty(expr));
                let len_ty = match *array_ty {
                    TypX::Array(_, len_ty) => len_ty.clone(),
                    _ => {
                        panic!("ExprKind::Repeat case expected TypX::Array");
                    }
                };
                mk_exp(ExpX::ArrayRepeat(exp.expect("expr"), len_ty))
            }
        }
        ExprKind::Cast(source, _) => {
            let source = erase_expr(ctxt, state, expect_spec, source);
            erase_spec_exps(ctxt, state, expr, vec![source])
        }
        ExprKind::AddrOf(BorrowKind::Ref, mutability, e) => {
            let exp = erase_expr(ctxt, state, expect_spec, e);
            if expect_spec {
                erase_spec_exps(ctxt, state, expr, vec![exp])
            } else {
                mk_exp(ExpX::AddrOf(*mutability, exp.expect("expr")))
            }
        }
        ExprKind::Unary(op, e1) => {
            let exp1 = erase_expr(ctxt, state, expect_spec, e1);
            match op {
                UnOp::Deref if !expect_spec => {
                    if auto_deref_supported_for_ty(ctxt.tcx, &ctxt.types().node_type(e1.hir_id))
                        || !ctxt.types().is_method_call(expr)
                    {
                        mk_exp(ExpX::Deref(exp1.expect("expr")))
                    } else {
                        let fn_def_id = ctxt
                            .types()
                            .type_dependent_def_id(expr.hir_id)
                            .expect("`deref` method ID not found");
                        erase_call(
                            ctxt,
                            state,
                            expect_spec,
                            expr,
                            None,
                            Some(fn_def_id),
                            ctxt.types().node_args(expr.hir_id),
                            expr.span,
                            Some(e1),
                            &[],
                            true,
                            false,
                        )
                    }
                }
                _ => erase_spec_exps(ctxt, state, expr, vec![exp1]),
            }
        }
        ExprKind::Binary(op, e1, e2) => {
            let mut exp1 = erase_expr(ctxt, state, expect_spec, e1);
            let mut exp2 = erase_expr(ctxt, state, expect_spec, e2);
            let use_ref = matches!(
                op.node,
                BinOpKind::Eq
                    | BinOpKind::Ne
                    | BinOpKind::Gt
                    | BinOpKind::Ge
                    | BinOpKind::Lt
                    | BinOpKind::Le
            );
            if use_ref {
                if let Some(e) = exp1 {
                    exp1 = Some(Box::new((e.0, ExpX::AddrOf(Mutability::Not, e))))
                }
                if let Some(e) = exp2 {
                    exp2 = Some(Box::new((e.0, ExpX::AddrOf(Mutability::Not, e))))
                }
            }
            erase_spec_exps(ctxt, state, expr, vec![exp1, exp2])
        }
        ExprKind::Index(e1, e2, _span) => {
            let exp1 = erase_expr(ctxt, state, expect_spec, e1);
            let exp2 = erase_expr(ctxt, state, expect_spec, e2);
            if expect_spec {
                erase_spec_exps(ctxt, state, expr, vec![exp1, exp2])
            } else {
                let ty1 = erase_ty(ctxt, state, &ctxt.types().expr_ty_adjusted(e1));
                let ty2 = erase_ty(ctxt, state, &ctxt.types().expr_ty_adjusted(e2));
                let ty = erase_ty(ctxt, state, &ctxt.types().node_type(expr.hir_id));
                mk_exp(ExpX::Index(ty1, ty2, ty, exp1.expect("expr"), exp2.expect("expr")))
            }
        }
        ExprKind::Field(e1, field) => {
            let exp1 = erase_expr(ctxt, state, expect_spec, e1);
            if expect_spec {
                erase_spec_exps(ctxt, state, expr, vec![exp1])
            } else {
                let field_id = state.field(field.to_string());
                mk_exp(ExpX::Field(exp1.expect("expr"), field_id))
            }
        }
        ExprKind::Assign(e1, e2, _span) => {
            let mode1 = ctxt.var_modes[&e1.hir_id];
            if mode1 == Mode::Spec {
                let exp1 = erase_expr(ctxt, state, true, e1);
                let exp2 = erase_expr(ctxt, state, true, e2);
                erase_spec_exps(ctxt, state, expr, vec![exp1, exp2])
            } else {
                let exp1 = erase_expr(ctxt, state, false, e1);
                let exp2 = erase_expr(ctxt, state, false, e2);
                mk_exp(ExpX::Assign(exp1.expect("expr"), force_exp(exp2, e2.span)))
            }
        }
        ExprKind::AssignOp(_op, e1, e2) => {
            let mode1 = ctxt.var_modes[&e1.hir_id];
            if mode1 == Mode::Spec {
                let exp1 = erase_expr(ctxt, state, true, e1);
                let exp2 = erase_expr(ctxt, state, true, e2);
                erase_spec_exps(ctxt, state, expr, vec![exp1, exp2])
            } else {
                // REVIEW:
                // Right now, we duplicate exp1; this is ok because we only consider one kind of
                // expressions on the lhs: ExprX::VarLoc(_). When we add support
                // for more kinds, this may cause the borrow-checker to report errors in places
                // where it shouldn't (borrow-checking is still sound, but innacurate for these
                // expressions).
                let exp1 = erase_expr(ctxt, state, false, e1);
                let exp2 = erase_expr(ctxt, state, false, e2);
                let expr_typ =
                    |state: &mut State| erase_ty(ctxt, state, &ctxt.types().node_type(e1.hir_id));
                let exp3 = erase_spec_exps_typ(
                    ctxt,
                    state,
                    expr.span,
                    expr_typ,
                    vec![exp1.clone(), exp2],
                    false,
                );
                mk_exp(ExpX::Assign(exp1.expect("expr"), exp3.expect("expr")))
            }
        }
        ExprKind::If(cond, lhs, rhs) => {
            let cond_spec = ctxt.condition_modes[&expr.hir_id] == Mode::Spec;
            let cond = cond.peel_drop_temps();
            match cond.kind {
                ExprKind::Let(LetExpr { pat, init: src_expr, .. }) => {
                    let arm1 = (Some(pat.to_owned()), &None, Some(*lhs));
                    let arm2 = (None, &None, *rhs);
                    erase_match(ctxt, state, expect_spec, expr, src_expr, vec![arm1, arm2])
                }
                _ => {
                    let ec = erase_expr(ctxt, state, cond_spec, cond);
                    let e1 = erase_expr(ctxt, state, expect_spec, lhs);
                    let e2 = match rhs {
                        None => None,
                        Some(rhs) => erase_expr(ctxt, state, expect_spec, rhs),
                    };
                    match (expect_spec, e1, e2) {
                        (true, None, None) => erase_spec_exps(ctxt, state, expr, vec![ec]),
                        (_, e1, e2) => {
                            let c = match ec {
                                None => mk_exp1(ExpX::Op(vec![], TypX::mk_bool())),
                                Some(e) => e,
                            };
                            let e1 = force_block(e1, lhs.span);
                            let e2 = force_block(e2, lhs.span);
                            mk_exp(ExpX::If(c, e1, e2))
                        }
                    }
                }
            }
        }
        ExprKind::Match(cond, arms, _match_source) => {
            let arms_vec = arms.iter().map(|a| (Some(a.pat), &a.guard, Some(a.body))).collect();
            erase_match(ctxt, state, expect_spec, expr, cond, arms_vec)
        }
        ExprKind::Loop(
            Block {
                stmts: [],
                expr: Some(Expr { kind: ExprKind::If(cond, body, _other), .. }),
                ..
            },
            label,
            rustc_hir::LoopSource::While,
            _span,
        ) => {
            let c = erase_expr(ctxt, state, false, cond).expect("expr");
            let b = force_block(erase_expr(ctxt, state, false, body), body.span);
            let label = label.map(|l| state.lifetime((l.ident.to_string(), None)));
            mk_exp(ExpX::While(c, b, label))
        }
        ExprKind::Loop(block, label, _source, _span) => {
            let b = force_block(erase_block(ctxt, state, false, block), block.span);
            let label = label.map(|l| state.lifetime((l.ident.to_string(), None)));
            mk_exp(ExpX::Loop(b, label))
        }
        ExprKind::Break(dest, None) => {
            let label = dest.label.map(|l| state.lifetime((l.ident.to_string(), None)));
            mk_exp(ExpX::Break(label))
        }
        ExprKind::Continue(dest) => {
            let label = dest.label.map(|l| state.lifetime((l.ident.to_string(), None)));
            mk_exp(ExpX::Continue(label))
        }
        ExprKind::Ret(None) => mk_exp(ExpX::Ret(None)),
        ExprKind::Ret(Some(expr)) => {
            let exp = erase_expr(ctxt, state, ctxt.ret_spec.expect("ret_spec"), expr);
            mk_exp(ExpX::Ret(exp))
        }
        ExprKind::Closure(_) => {
            Some(erase_expr_closure(ctxt, state, expect_spec, Mode::Exec, expr))
        }
        ExprKind::Block(block, None) => {
            let attrs = ctxt.tcx.hir_attrs(expr.hir_id);
            if crate::rust_to_vir_expr::attrs_is_invariant_block(attrs).expect("attrs") {
                return Some(erase_inv_block(ctxt, state, expr.span, block));
            }
            let g_attr = get_ghost_block_opt(ctxt.tcx.hir_attrs(expr.hir_id));
            let keep = match g_attr {
                Some(GhostBlockAttr::Proof) => true,
                Some(GhostBlockAttr::Tracked) => true,
                Some(GhostBlockAttr::GhostWrapped) => false,
                Some(GhostBlockAttr::TrackedWrapped) => true,
                Some(GhostBlockAttr::Wrapper) => panic!(),
                None => true,
            };
            if keep { erase_block(ctxt, state, expect_spec, block) } else { None }
        }
        _ => {
            dbg!(&expr);
            panic!()
        }
    }
}

fn erase_expr_closure<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expect_spec: bool,
    body_mode: Mode,
    expr: &Expr<'tcx>,
) -> Exp {
    match &expr.kind {
        ExprKind::Closure(Closure { capture_clause: capture_by, body: body_id, .. }) => {
            let mut params: Vec<(Span, Id, Typ)> = Vec::new();
            let body = ctxt.tcx.hir_body(*body_id);
            let ps = &body.params;
            for p in ps.iter() {
                let pat_var = crate::rust_to_vir_expr::pat_to_var(p.pat).expect("pat_to_var");
                let (x, local_id) = match &pat_var {
                    vir::ast::VarIdent(x, vir::ast::VarIdentDisambiguate::RustcId(local_id)) => {
                        (x, local_id)
                    }
                    _ => panic!("pat_to_var"),
                };
                let x = state.local(x.to_string(), *local_id);
                let typ = erase_ty(ctxt, state, &ctxt.types().node_type(p.hir_id));
                params.push((p.pat.span, x, typ));
            }
            let body_exp = if body_mode == Mode::Spec {
                let spec_exp = erase_expr(ctxt, state, true, &body.value);
                let ty = ctxt.types().node_type(body.value.hir_id);
                let typ = erase_ty(ctxt, state, &ty);
                Some(erase_spec_exps_force_typ(ctxt, state, body.value.span, typ, vec![spec_exp]))
            } else {
                erase_expr(ctxt, state, expect_spec, &body.value)
            };
            let body_exp = force_block(body_exp, body.value.span);
            Box::new((expr.span, ExpX::Closure(*capture_by, None, params, body_exp)))
        }
        _ => panic!("expected closure"),
    }
}

fn erase_block<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    expect_spec: bool,
    block: &Block<'tcx>,
) -> Option<Exp> {
    let mk_exp = |e: ExpX| Some(Box::new((block.span, e)));
    assert!(matches!(block.rules, BlockCheckMode::DefaultBlock | BlockCheckMode::UnsafeBlock(_)));
    assert!(!block.targeted_by_break);
    let mut stms: Vec<Stm> = Vec::new();
    for stmt in block.stmts {
        stms.extend(erase_stmt(ctxt, state, stmt));
    }
    let e = block.expr.and_then(|e| erase_expr(ctxt, state, expect_spec, e));
    if stms.len() > 0 || e.is_some() { mk_exp(ExpX::Block(stms, e)) } else { None }
}

fn erase_stmt<'tcx>(ctxt: &Context<'tcx>, state: &mut State, stmt: &Stmt<'tcx>) -> Vec<Stm> {
    match &stmt.kind {
        StmtKind::Expr(e) | StmtKind::Semi(e) => {
            if let Some(e) = erase_expr(ctxt, state, true, e) {
                vec![Box::new((stmt.span, StmX::Expr(e)))]
            } else {
                vec![]
            }
        }
        StmtKind::Let(LetStmt { pat, ty: _, init, els, hir_id, .. }) => {
            let mode = ctxt.var_modes[&pat.hir_id];
            if mode != Mode::Exec && els.is_some() {
                panic!("let-else is not supported in spec");
            }
            if mode == Mode::Spec {
                if let Some(init) = init {
                    if let Some(e) = erase_expr(ctxt, state, true, init) {
                        vec![Box::new((stmt.span, StmX::Expr(e)))]
                    } else {
                        vec![]
                    }
                } else {
                    vec![]
                }
            } else {
                let pat: Pattern = erase_pat(ctxt, state, pat);
                let typ = erase_ty(ctxt, state, &ctxt.types().node_type(*hir_id));
                let init_exp =
                    if let Some(init) = init { erase_expr(ctxt, state, false, init) } else { None };
                let els_expr = if let Some(els) = els {
                    erase_block(ctxt, state, false, &els)
                        .or_else(|| Some(Box::new((els.span, ExpX::Panic))))
                } else {
                    None
                };
                vec![Box::new((stmt.span, StmX::Let(pat, typ, init_exp, els_expr)))]
            }
        }
        StmtKind::Item(item_id) => {
            let item = ctxt.tcx.hir_item(*item_id);
            if matches!(&item.kind, ItemKind::Use(..) | ItemKind::Macro(..)) {
                return vec![];
            }
            panic!("unexpected statement");
        }
    }
}

fn erase_const_or_static<'tcx>(
    krate: Option<&'tcx Crate<'tcx>>,
    ctxt: &mut Context<'tcx>,
    state: &mut State,
    span: Span,
    id: DefId,
    external_body: bool,
    body_id: Option<&BodyId>,
    is_static: bool,
) {
    // When importing a const/static, we expect both to be None.
    // Otherwise, both should be Some.
    assert!(krate.is_none() == body_id.is_none());
    let path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);
    if let Some(s) = path.segments.last() {
        if s.to_string().starts_with("_DERIVE_builtin_Structural_FOR_") {
            return;
        }
    }
    let fun_name = Arc::new(FunX { path });
    if let Some(Some(f_vir)) = ctxt.functions.get(&fun_name) {
        if f_vir.x.mode == Mode::Spec && f_vir.x.ret.x.mode == Mode::Spec {
            return;
        }
        if let Some(body_id) = body_id {
            let types = ctxt.tcx.typeck(body_id.hir_id.owner.def_id);
            ctxt.types_opt = Some(types);
            ctxt.ret_spec = Some(f_vir.x.ret.x.mode == Mode::Spec);
        }

        let name = state.fun_name(&fun_name);
        let ty = ctxt.tcx.type_of(id).skip_binder();
        let typ = erase_ty(ctxt, state, &ty);
        let body = if let Some(body_id) = body_id {
            Some(crate::rust_to_vir_func::find_body_krate(krate.expect("krate"), body_id))
        } else {
            None
        };
        let body_exp = if body.is_none() || external_body {
            Box::new((span, ExpX::Panic))
        } else {
            let body = &body.expect("body");
            state.enclosing_fun_id = Some(id);
            let body_exp = erase_expr(ctxt, state, false, &body.value).expect("const body");
            state.enclosing_fun_id = None;
            body_exp
        };

        let mut return_typ = typ;
        let body_span = body_exp.0;
        let mut body = Box::new((body_span, ExpX::Block(vec![], Some(body_exp))));

        if is_static {
            // For static `static x: T` we change it to
            // fn x() -> &'static T {
            //     static_ref(body)
            // }

            let target = Box::new((
                body_span,
                ExpX::Var(Id::new(IdKind::Builtin, 0, "static_ref".to_string())),
            ));
            body = Box::new((body_span, ExpX::Call(target, vec![return_typ.clone()], vec![body])));
            body = Box::new((body_span, ExpX::Block(vec![], Some(body))));
            return_typ = Box::new(TypX::Ref(
                return_typ,
                Some(Id::new(IdKind::Builtin, 0, "'static".to_string())),
                Mutability::Not,
            ));
        }

        // Turn const decl into fn decl so we can use the non-const ExpX::Op in the body:
        let decl = FunDecl {
            sig_span: span,
            name_span: span,
            name,
            generic_params: vec![],
            generic_bounds: vec![],
            params: vec![],
            ret: Some((None, return_typ)),
            body: body,
        };
        state.fun_decls.push(decl);
        ctxt.types_opt = None;
        ctxt.ret_spec = None;
    }
}

fn lifetime_key<'tcx>(ctxt: &Context<'tcx>, def_id: DefId) -> (String, Option<u32>) {
    let def_path = ctxt.tcx.def_path(def_id);
    let path_name = &def_path.data.last().unwrap();
    (path_name.data.get_opt_name().unwrap().to_string(), Some(path_name.disambiguator))
}

fn erase_mir_bound<'a, 'tcx>(
    ctxt: &Context<'tcx>,
    state: &'a mut State,
    id: DefId,
    args: &[rustc_middle::ty::GenericArg<'tcx>],
) -> Option<Bound> {
    let tcx = ctxt.tcx;
    erase_trait(ctxt, state, id);
    let trait_path = def_id_to_vir_path(tcx, &ctxt.verus_items, id);
    let rust_item = verus_items::get_rust_item(ctxt.tcx, id);
    let verus_item = ctxt.verus_items.id_to_name.get(&id);
    if Some(id) == tcx.lang_items().copy_trait() {
        Some(Bound::Copy)
    } else if Some(id) == tcx.lang_items().clone_trait() {
        Some(Bound::Clone)
    } else if Some(id) == tcx.lang_items().sized_trait() {
        Some(Bound::Sized)
    } else if Some(RustItem::Allocator) == rust_item {
        Some(Bound::Allocator)
    } else if Some(id) == tcx.lang_items().pointee_trait() {
        // The Rust documentation says Pointee "is automatically implemented for every type",
        // so it's a special case here
        Some(Bound::Pointee)
    } else if vir::ast_util::path_as_friendly_rust_name(&trait_path) == "core::ptr::metadata::Thin"
    {
        // "Thin" is a trait alias for Pointee (special case since we don't support trait aliases)
        Some(Bound::Thin)
    } else if Some(&VerusItem::External(ExternalItem::ProofFnOnce)) == verus_item {
        Some(Bound::ProofFn(ClosureKind::FnOnce))
    } else if Some(&VerusItem::External(ExternalItem::ProofFnMut)) == verus_item {
        Some(Bound::ProofFn(ClosureKind::FnMut))
    } else if Some(&VerusItem::External(ExternalItem::ProofFn)) == verus_item {
        Some(Bound::ProofFn(ClosureKind::Fn))
    } else if state.trait_decl_set.contains(&trait_path) {
        let (args, _) = erase_generic_args(ctxt, state, args, true);
        let trait_path = state.trait_name(&trait_path);
        Some(Bound::Trait { trait_path, args, equality: None })
    } else {
        None
    }
}

fn erase_mir_predicates<'a, 'tcx>(
    ctxt: &Context<'tcx>,
    state: &'a mut State,
    mir_predicates: impl Iterator<Item = rustc_middle::ty::Clause<'tcx>>,
    generic_bounds: &mut Vec<GenericBound>,
) where
    'tcx: 'a,
{
    let tcx = ctxt.tcx;
    let mut fn_traits: Vec<(Typ, Vec<Id>, ClosureKind)> = Vec::new();
    let mut fn_projections: HashMap<Typ, (Typ, Typ)> = HashMap::new();
    for pred in mir_predicates {
        let mut bound_vars: Vec<Id> = Vec::new();
        for x in pred.kind().bound_vars().iter() {
            let a = match x {
                BoundVariableKind::Region(BoundRegionKind::Named(a, _)) => a,
                _ => panic!("expected region"),
            };
            let id = state.lifetime(lifetime_key(ctxt, a));
            state.rename_bound_for.push(id);
            let id = state.lifetime(lifetime_key(ctxt, a));
            bound_vars.push(id);
        }
        match pred.kind().skip_binder() {
            ClauseKind::RegionOutlives(pred) => {
                let x = erase_hir_region(ctxt, state, &pred.0.kind()).expect("bound");
                let typ = Box::new(TypX::TypParam(x));
                let bound = erase_hir_region(ctxt, state, &pred.1.kind()).expect("bound");
                let generic_bound = GenericBound { typ, bound_vars, bound: Bound::Id(bound) };
                generic_bounds.push(generic_bound);
            }
            ClauseKind::TypeOutlives(pred) => {
                let typ = erase_ty(ctxt, state, &pred.0);
                let bound = erase_hir_region(ctxt, state, &pred.1.kind()).expect("bound");
                let generic_bound = GenericBound { typ, bound_vars, bound: Bound::Id(bound) };
                generic_bounds.push(generic_bound);
            }
            ClauseKind::Trait(pred) => {
                let typ = erase_ty(ctxt, state, &pred.trait_ref.args[0].expect_ty());
                let id = pred.trait_ref.def_id;
                let bound = erase_mir_bound(ctxt, state, id, pred.trait_ref.args);
                if let Some(bound) = bound {
                    let generic_bound =
                        GenericBound { typ: typ.clone(), bound_vars: bound_vars.clone(), bound };
                    generic_bounds.push(generic_bound);
                }
                let kind = if Some(id) == tcx.lang_items().fn_trait() {
                    Some(ClosureKind::Fn)
                } else if Some(id) == tcx.lang_items().fn_mut_trait() {
                    Some(ClosureKind::FnMut)
                } else if Some(id) == tcx.lang_items().fn_once_trait() {
                    Some(ClosureKind::FnOnce)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    fn_traits.push((typ, bound_vars, kind));
                }
            }
            ClauseKind::Projection(pred) => {
                if Some(pred.projection_term.def_id) == tcx.lang_items().fn_once_output() {
                    assert!(pred.projection_term.args.len() == 2);
                    let typ = erase_ty(ctxt, state, &pred.projection_term.args[0].expect_ty());
                    let mut fn_params = match pred.projection_term.args[1].unpack() {
                        GenericArgKind::Type(ty) => erase_ty(ctxt, state, &ty),
                        _ => panic!("unexpected fn projection"),
                    };
                    if !matches!(*fn_params, TypX::Tuple(_)) {
                        fn_params = Box::new(TypX::Tuple(vec![fn_params]));
                    }
                    let fn_ret = if let TermKind::Ty(ty) = pred.term.unpack() {
                        erase_ty(ctxt, state, &ty)
                    } else {
                        panic!("fn_ret");
                    };
                    fn_projections.insert(typ, (fn_params, fn_ret)).map(|_| panic!("{:?}", pred));
                } else {
                    let typ0 = erase_ty(ctxt, state, &pred.projection_term.args[0].expect_ty());
                    let typ_eq = if let TermKind::Ty(ty) = pred.term.unpack() {
                        erase_ty(ctxt, state, &ty)
                    } else {
                        panic!("should have been disallowed by rust_verify_base.rs");
                    };
                    let trait_def_id = pred.projection_term.trait_def_id(tcx);
                    let item_def_id = pred.projection_term.def_id;
                    let assoc_item = tcx.associated_item(item_def_id);
                    let projection_generics = ctxt.tcx.generics_of(item_def_id);
                    let n = pred.projection_term.args.len() - projection_generics.own_params.len();
                    let mut bound =
                        erase_mir_bound(ctxt, state, trait_def_id, &pred.projection_term.args[..n])
                            .expect("bound");
                    let (x_args, _) =
                        erase_generic_args(ctxt, state, &pred.projection_term.args[n..], true);
                    let x_args = x_args.into_iter().map(|a| a.as_lifetime()).collect();
                    if let Bound::Trait { trait_path: _, args: _, equality } = &mut bound {
                        assert!(equality.is_none());
                        let name = state.typ_param(assoc_item.name().to_ident_string(), None);
                        *equality = Some((name, x_args, typ_eq));
                    } else if matches!(&bound, Bound::Pointee | Bound::Thin) {
                        // keep as is
                    } else {
                        panic!("unexpected bound")
                    }
                    let generic_bound = GenericBound { typ: typ0, bound_vars, bound };
                    generic_bounds.push(generic_bound);
                }
            }
            ClauseKind::ConstArgHasType(..) => {}
            _ => {
                panic!("unexpected bound")
            }
        }
        for _ in pred.kind().bound_vars().iter() {
            state.rename_bound_for.pop();
        }
    }
    for (typ, bound_vars, kind) in fn_traits.into_iter() {
        let (params, ret) = fn_projections.remove(&typ).expect("fn_projections");
        let generic_bound = GenericBound { typ, bound_vars, bound: Bound::Fn(kind, params, ret) };
        generic_bounds.push(generic_bound);
    }
}

fn erase_mir_generics<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    id: DefId,
    id_is_fn: bool,
    lifetimes: &mut Vec<GenericParam>,
    typ_params: &mut Vec<GenericParam>,
    generic_bounds: &mut Vec<GenericBound>,
) {
    let mir_generics = ctxt.tcx.generics_of(id);
    let mir_predicates = ctxt.tcx.predicates_of(id);
    if id_is_fn {
        let mir_ty = ctxt.tcx.type_of(id).skip_binder();
        if let TyKind::FnDef(..) = mir_ty.kind() {
            for bv in mir_ty.fn_sig(ctxt.tcx).bound_vars().iter() {
                if let BoundVariableKind::Region(BoundRegionKind::Named(a, _)) = bv {
                    let name = state.lifetime(lifetime_key(ctxt, a));
                    lifetimes.push(GenericParam { name, const_typ: None });
                }
            }
        }
    }
    for gparam in &mir_generics.own_params {
        match gparam.kind {
            GenericParamDefKind::Lifetime => {
                let name = state.lifetime((gparam.name.to_string(), Some(gparam.index)));
                lifetimes.push(GenericParam { name, const_typ: None });
            }
            GenericParamDefKind::Type { .. } => {
                let name = state.typ_param(gparam.name.to_string(), Some(gparam.index));
                typ_params.push(GenericParam { name, const_typ: None });
            }
            GenericParamDefKind::Const { has_default: _, .. } => {
                let name = state.typ_param(gparam.name.to_string(), None);
                let t = erase_ty(ctxt, state, &ctxt.tcx.type_of(gparam.def_id).skip_binder());
                typ_params.push(GenericParam { name, const_typ: Some(t) });
            }
        }
    }
    erase_mir_predicates(
        ctxt,
        state,
        mir_predicates.predicates.iter().map(|(c, _)| *c),
        generic_bounds,
    );
}

fn erase_fn_common<'tcx>(
    krate: Option<&'tcx Crate<'tcx>>,
    ctxt: &mut Context<'tcx>,
    state: &mut State,
    name_span: Span,
    id: DefId,
    sig: Option<&FnSig<'tcx>>,
    sig_span: Span,
    impl_generics: Option<DefId>,
    empty_body: bool,
    external_body: bool,
    body_id: Option<&BodyId>,
) {
    if ctxt.ignored_functions.contains(&id) {
        return;
    }

    let mut path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);

    if let Some(local_id) = id.as_local() {
        let hir_id = ctxt.tcx.local_def_id_to_hir_id(local_id);
        let attrs = ctxt.tcx.hir_attrs(hir_id);
        let vattrs = get_verifier_attrs(attrs, None).expect("get_verifier_attrs");

        if vattrs.unerased_proxy {
            path = crate::rust_to_vir_func::fixup_unerased_proxy_path(&path, sig_span)
                .expect("fixup_unerased_proxy_path");
        }
    }

    let is_verus_spec = path.segments.last().expect("segment.last").starts_with(VERUS_SPEC);
    // TODO let is_verus_reveal = **path.segments.last().expect("segments.last") == VERUS_REVEAL_INTERNAL;
    if is_verus_spec {
        return;
    }
    let fun_name = Arc::new(FunX { path: path.clone() });
    if let Some(f_vir) = &ctxt.functions[&fun_name] {
        if f_vir.x.mode == Mode::Spec && f_vir.x.ret.x.mode == Mode::Spec {
            return;
        }
        if let Some(body_id) = body_id {
            let types = ctxt.tcx.typeck(body_id.hir_id.owner.def_id);
            ctxt.types_opt = Some(types);
            ctxt.ret_spec = Some(f_vir.x.ret.x.mode == Mode::Spec);
        }

        let expect_spec = f_vir.x.ret.x.mode == Mode::Spec;
        let body = if let Some(body_id) = body_id {
            Some(crate::rust_to_vir_func::find_body_krate(krate.expect("krate"), body_id))
        } else {
            None
        };
        let mut body_exp = if body.is_none() || external_body {
            force_block(Some(Box::new((sig_span, ExpX::Panic))), sig_span)
        } else {
            let body = &body.expect("body");
            state.enclosing_fun_id = Some(id);
            let body_exp = erase_expr(ctxt, state, expect_spec, &body.value);
            state.enclosing_fun_id = None;
            if empty_body {
                if let Some(_) = body_exp {
                    panic!("expected empty method body")
                } else {
                    force_block(Some(Box::new((sig_span, ExpX::Panic))), sig_span)
                }
            } else {
                force_block(body_exp, body.value.span)
            }
        };
        let fn_sig = ctxt.tcx.fn_sig(id);
        let fn_sig = fn_sig.skip_binder();
        state.rename_count += 1;
        let name = state.fun_name(&fun_name);
        let inputs = &fn_sig.inputs().skip_binder();
        assert!(inputs.len() == f_vir.x.params.len());
        let params_info: Vec<(Option<Span>, bool)> = if let Some(body) = body {
            assert!(inputs.len() == body.params.len());
            body.params
                .iter()
                .map(|p| {
                    let is_mut_var = match p.pat.kind {
                        PatKind::Binding(BindingMode(_, mutability), _, _, _) => {
                            mutability == rustc_hir::Mutability::Mut
                        }
                        _ => panic!("expected binding pattern"),
                    };
                    (Some(p.pat.span), is_mut_var)
                })
                .collect()
        } else {
            inputs.iter().map(|_| (None, false)).collect()
        };
        let mut lifetimes: Vec<GenericParam> = Vec::new();
        let mut typ_params: Vec<GenericParam> = Vec::new();
        let mut generic_bounds: Vec<GenericBound> = Vec::new();
        if let Some(impl_id) = impl_generics {
            erase_mir_generics(
                ctxt,
                state,
                impl_id,
                false,
                &mut lifetimes,
                &mut typ_params,
                &mut generic_bounds,
            );
        }
        erase_mir_generics(
            ctxt,
            state,
            id,
            true,
            &mut lifetimes,
            &mut typ_params,
            &mut generic_bounds,
        );

        state.enclosing_fun_id = Some(id);
        let mut params: Vec<Param> = Vec::new();
        for ((input, param), param_info) in
            inputs.iter().zip(f_vir.x.params.iter()).zip(params_info.iter())
        {
            let name =
                if let Some((_, name)) = &param.x.unwrapped_info { name } else { &param.x.name };
            let (x, local_id) = match name {
                vir::ast::VarIdent(x, vir::ast::VarIdentDisambiguate::RustcId(local_id)) => {
                    (x, local_id)
                }
                _ => panic!("pat_to_var"),
            };
            let is_mut_var = param_info.1;
            let span = param_info.0;
            let name = state.local(x.to_string(), *local_id);
            let typ = if param.x.mode == Mode::Spec {
                TypX::mk_unit()
            } else {
                erase_ty(ctxt, state, input)
            };
            let new_param = Param { name, span, typ, is_mut_var };
            params.push(new_param);
        }
        let mut ret = if let Some(sig) = sig {
            match sig.decl.output {
                rustc_hir::FnRetTy::DefaultReturn(_) => None,
                rustc_hir::FnRetTy::Return(ty) => {
                    if f_vir.x.ret.x.mode == Mode::Spec {
                        None
                    } else {
                        Some((Some(ty.span), erase_ty(ctxt, state, &fn_sig.output().skip_binder())))
                    }
                }
            }
        } else {
            Some((None, erase_ty(ctxt, state, &fn_sig.output().skip_binder())))
        };

        if matches!(f_vir.x.item_kind, vir::ast::ItemKind::Static) {
            // For static `static x: T` we change it to
            // fn x() -> &'static T {
            //     static_ref(body)
            // }

            let (name, mut return_typ) = ret.clone().unwrap();

            let target = Box::new((
                sig_span,
                ExpX::Var(Id::new(IdKind::Builtin, 0, "static_ref".to_string())),
            ));
            body_exp =
                Box::new((sig_span, ExpX::Call(target, vec![return_typ.clone()], vec![body_exp])));
            body_exp = Box::new((sig_span, ExpX::Block(vec![], Some(body_exp))));

            return_typ = Box::new(TypX::Ref(
                return_typ,
                Some(Id::new(IdKind::Builtin, 0, "'static".to_string())),
                Mutability::Not,
            ));
            ret = Some((name, return_typ));
        }

        state.enclosing_fun_id = None;

        // Special case for trait with direct self argument
        if body.is_none() && inputs.len() > 0 {
            match inputs[0].kind() {
                TyKind::Param(p) if p.name == kw::SelfUpper => {
                    // Add Sized bound to make function declaration legal
                    let generic_bound = GenericBound {
                        typ: params[0].typ.clone(),
                        bound_vars: vec![],
                        bound: Bound::Sized,
                    };
                    generic_bounds.push(generic_bound);
                }
                _ => {}
            }
        }

        let decl = FunDecl {
            sig_span: sig_span,
            name_span,
            name,
            // lifetimes must precede typ_params
            generic_params: lifetimes.into_iter().chain(typ_params.into_iter()).collect(),
            generic_bounds,
            params,
            ret,
            body: body_exp,
        };
        state.fun_decls.push(decl);
        ctxt.types_opt = None;
        ctxt.ret_spec = None;
    }
}

fn import_fn<'tcx>(ctxt: &mut Context<'tcx>, state: &mut State, id: DefId) {
    erase_fn_common(
        None,
        ctxt,
        state,
        ctxt.tcx.def_ident_span(id).expect("function name span"),
        id,
        None,
        ctxt.tcx.def_span(id),
        ctxt.tcx.generics_of(id).parent,
        true,
        true,
        None,
    );
}

fn import_const_static<'tcx>(
    ctxt: &mut Context<'tcx>,
    state: &mut State,
    id: DefId,
    is_static: bool,
) {
    erase_const_or_static(
        None,
        ctxt,
        state,
        ctxt.tcx.def_ident_span(id).expect("const/static name span"),
        id,
        true,
        None,
        is_static,
    );
}

fn erase_fn<'tcx>(
    krate: &'tcx Crate<'tcx>,
    ctxt: &mut Context<'tcx>,
    state: &mut State,
    name_span: Span,
    id: DefId,
    sig: &FnSig<'tcx>,
    impl_generics: Option<DefId>,
    empty_body: bool,
    external_body: bool,
    body_id: Option<&BodyId>,
) {
    erase_fn_common(
        Some(krate),
        ctxt,
        state,
        name_span,
        id,
        Some(sig),
        sig.span,
        impl_generics,
        empty_body,
        external_body,
        body_id,
    );
}

fn erase_impl_assocs<'tcx>(ctxt: &Context<'tcx>, state: &mut State, impl_id: DefId) {
    let (name, _) = state.remaining_typs_needed_for_each_impl.remove(&impl_id).unwrap();
    let trait_ref = ctxt.tcx.impl_trait_ref(impl_id).expect("impl_trait_ref");
    let trait_id = trait_ref.skip_binder().def_id;
    let is_copy = Some(trait_id) == ctxt.tcx.lang_items().copy_trait();
    let is_clone = Some(trait_id) == ctxt.tcx.lang_items().clone_trait();
    let is_copy_or_clone = is_copy || is_clone;

    let span = ctxt.tcx.def_span(impl_id);

    let args = trait_ref.skip_binder().args;
    let (trait_typ_args, _) = erase_generic_args(ctxt, state, args, true);

    let mut lifetimes: Vec<GenericParam> = Vec::new();
    let mut typ_params: Vec<GenericParam> = Vec::new();
    let mut generic_bounds: Vec<GenericBound> = Vec::new();
    erase_mir_generics(
        ctxt,
        state,
        impl_id,
        false,
        &mut lifetimes,
        &mut typ_params,
        &mut generic_bounds,
    );
    let generic_params = lifetimes.into_iter().chain(typ_params.into_iter()).collect();
    let self_ty = ctxt.tcx.type_of(impl_id).skip_binder();
    let self_typ = erase_ty(ctxt, state, &self_ty);
    let trait_as_datatype = Box::new(TypX::Datatype(name.clone(), vec![], trait_typ_args));

    if is_copy_or_clone {
        if let TypX::Datatype(x, _, _) = &*self_typ {
            if x.kind != IdKind::Datatype {
                return;
            }
        } else {
            return;
        }
    }

    let mut assoc_typs: Vec<(Id, Vec<GenericParam>, Typ)> = Vec::new();
    for assoc_item in ctxt.tcx.associated_items(impl_id).in_definition_order() {
        match assoc_item.kind {
            rustc_middle::ty::AssocKind::Type { .. } => {
                let mut lifetimes: Vec<GenericParam> = Vec::new();
                let mut typ_params: Vec<GenericParam> = Vec::new();
                let mut generic_bounds: Vec<GenericBound> = Vec::new();
                erase_mir_generics(
                    ctxt,
                    state,
                    assoc_item.def_id,
                    false,
                    &mut lifetimes,
                    &mut typ_params,
                    &mut generic_bounds,
                );
                assert!(typ_params.len() == 0);
                let ty = ctxt.tcx.type_of(assoc_item.def_id).skip_binder();
                let typ = erase_ty(ctxt, state, &ty);
                let impl_name = state.typ_param(&assoc_item.name().to_string(), None);
                assoc_typs.push((impl_name, lifetimes, typ));
            }
            _ => (),
        }
    }

    let trait_polarity = ctxt.tcx.impl_polarity(impl_id);
    let trait_impl = TraitImpl {
        span: Some(span),
        generic_params,
        generic_bounds,
        self_typ: self_typ.clone(),
        trait_polarity,
        trait_as_datatype,
        assoc_typs,
        is_clone,
    };

    state.trait_impls.push(trait_impl);
}

fn erase_trait<'tcx>(ctxt: &Context<'tcx>, state: &mut State, trait_id: DefId) {
    if !state.reached.insert((None, trait_id)) {
        return;
    }
    // HACK: we cannot yet handle FnOnce::Output
    if let Some(fn_once) = ctxt.tcx.lang_items().fn_once_trait() {
        if trait_id == fn_once {
            return;
        }
    }
    let is_copy = Some(trait_id) == ctxt.tcx.lang_items().copy_trait();
    let is_clone = Some(trait_id) == ctxt.tcx.lang_items().clone_trait();
    let is_copy_or_clone = is_copy || is_clone;

    state.enclosing_trait_ids.push(trait_id);

    let path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, trait_id);

    let mut assoc_typs: Vec<(Id, Vec<GenericParam>, Vec<GenericBound>)> = Vec::new();
    let assoc_items = ctxt.tcx.associated_items(trait_id);
    state.inside_trait_decl += 1;
    for assoc_item in assoc_items.in_definition_order() {
        match assoc_item.kind {
            rustc_middle::ty::AssocKind::Const { .. } => {}
            rustc_middle::ty::AssocKind::Fn { .. } => {}
            rustc_middle::ty::AssocKind::Type { .. } => {
                let mut lifetimes: Vec<GenericParam> = Vec::new();
                let mut typ_params: Vec<GenericParam> = Vec::new();
                let mut generic_bounds = Vec::new();
                erase_mir_generics(
                    ctxt,
                    state,
                    assoc_item.def_id,
                    false,
                    &mut lifetimes,
                    &mut typ_params,
                    &mut generic_bounds,
                );
                assert!(generic_bounds.len() == 0); // if this doesn't hold, need to review
                let mut generic_bounds = Vec::new();
                let mir_predicates = ctxt.tcx.item_bounds(assoc_item.def_id);
                erase_mir_predicates(
                    ctxt,
                    state,
                    mir_predicates.skip_binder().iter(),
                    &mut generic_bounds,
                );
                assert!(typ_params.len() == 0);
                assoc_typs.push((
                    state.typ_param(assoc_item.name().to_ident_string(), None),
                    lifetimes,
                    generic_bounds,
                ));
            }
        }
    }

    // We only need traits with associated type declarations or Copy,
    // or traits that extend traits with associated type declarations or Copy.
    // First, check our own trait bounds to catch anything we might be extending
    // that has associated types.
    // (Note 1: this is an overapproximation, since we only really need bounds on Self,
    // but it's unlikely to matter much.)
    // (Note 2: if we allow cycles between a trait and its supertraits, we'll need a more
    // sophisticated algorithm.)
    let mut supertrait_may_have_assoc_types_or_copy = false;
    if is_copy_or_clone {
        supertrait_may_have_assoc_types_or_copy = true;
    }
    for (pred, _) in ctxt.tcx.predicates_of(trait_id).predicates.iter() {
        match (pred.kind().skip_binder(), &pred.kind().bound_vars()[..]) {
            (ClauseKind::Trait(pred), _bound_vars) => {
                let id = pred.trait_ref.def_id;
                erase_trait(ctxt, state, id);
                let trait_path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);
                if state.trait_decl_set.contains(&trait_path)
                    || Some(id) == ctxt.tcx.lang_items().copy_trait()
                {
                    supertrait_may_have_assoc_types_or_copy = true;
                }
            }
            _ => {}
        }
    }

    if supertrait_may_have_assoc_types_or_copy || assoc_typs.len() > 0 {
        let name = if is_copy {
            Id::new(IdKind::Builtin, 0, "Copy".to_owned())
        } else if is_clone {
            Id::new(IdKind::Builtin, 0, "Clone".to_owned())
        } else {
            state.trait_name(&path)
        };
        let mut lifetimes: Vec<GenericParam> = Vec::new();
        let mut typ_params: Vec<GenericParam> = Vec::new();
        let mut generic_bounds: Vec<GenericBound> = Vec::new();
        erase_mir_generics(
            ctxt,
            state,
            trait_id,
            false,
            &mut lifetimes,
            &mut typ_params,
            &mut generic_bounds,
        );
        typ_params.remove(0); // remove Self type parameter
        let generic_params = lifetimes.into_iter().chain(typ_params.into_iter()).collect();

        if !is_copy_or_clone {
            let decl = TraitDecl { name: name.clone(), generic_params, generic_bounds, assoc_typs };
            state.trait_decl_set.insert(path.clone());
            state.trait_decls.push(decl);
        }

        'imp: for impl_id in ctxt.tcx.all_impls(trait_id) {
            let mut datatypes: Vec<DefId> = Vec::new();
            let trait_ref = ctxt.tcx.impl_trait_ref(impl_id).expect("impl_trait_ref");
            for ty in trait_ref.skip_binder().args.types() {
                let result = collect_unreached_datatypes(ctxt, state, &mut datatypes, &ty);
                if result.is_err() {
                    continue 'imp;
                }
                for t in &datatypes {
                    state
                        .typs_used_in_trait_impls_reverse_map
                        .entry(*t)
                        .or_insert_with(|| Vec::new())
                        .push(impl_id);
                }
            }
            state
                .remaining_typs_needed_for_each_impl
                .insert(impl_id, (name.clone(), datatypes))
                .map(|_| panic!("already inserted"));
            state.reach_impl_assoc(impl_id);
        }
    }
    state.inside_trait_decl -= 1;

    assert!(state.enclosing_trait_ids.pop().is_some());
}

fn erase_trait_item<'tcx>(
    krate: &'tcx Crate<'tcx>,
    ctxt: &mut Context<'tcx>,
    state: &mut State,
    mut trait_id: DefId,
    ex_trait_id_for: Option<DefId>,
    items: &[TraitItemRef],
) {
    let tcx = ctxt.tcx;
    if let Some(ex_trait_id_for) = ex_trait_id_for {
        trait_id = ex_trait_id_for;
    }
    for trait_item_ref in items {
        let mut trait_item = tcx.hir_trait_item(trait_item_ref.id);
        let TraitItem { ident, owner_id, .. } = trait_item;
        if let Some(ex_trait_id_for) = ex_trait_id_for {
            let assoc_item = tcx.associated_item(owner_id.to_def_id());
            let ex_assoc_items = tcx.associated_items(ex_trait_id_for);
            let ex_assoc_item = ex_assoc_items.find_by_ident_and_kind(
                tcx,
                *ident,
                assoc_item.as_tag(),
                ex_trait_id_for,
            );
            if let Some(ex_assoc_item) = ex_assoc_item {
                let local_id = ex_assoc_item
                    .def_id
                    .as_local()
                    .expect("erase_trait_item only called on locals");
                let hir_id = tcx.local_def_id_to_hir_id(local_id);
                trait_item = tcx.hir_trait_item(rustc_hir::TraitItemId { owner_id: hir_id.owner });
            } else {
                continue;
            }
        }
        let TraitItem { ident, owner_id, generics: _, kind, span: _, defaultness: _ } = trait_item;
        match kind {
            TraitItemKind::Fn(sig, fun) => {
                let body_id = match (fun, ex_trait_id_for) {
                    (TraitFn::Provided(body_id), None) => Some(body_id),
                    (TraitFn::Required(..), None) => None,
                    (_, Some(_)) => None,
                };
                let id = owner_id.to_def_id();

                let attrs = ctxt.tcx.hir_attrs(trait_item.hir_id());
                let vattrs = get_verifier_attrs(attrs, None).expect("get_verifier_attrs");

                erase_fn(
                    krate,
                    ctxt,
                    state,
                    ident.span,
                    id,
                    sig,
                    Some(trait_id),
                    body_id.is_none(),
                    vattrs.external_body,
                    body_id,
                );
            }
            TraitItemKind::Type(_bounds, None) => {}
            _ => panic!("unexpected trait item"),
        }
    }
}

fn erase_impl<'tcx>(
    krate: &'tcx Crate<'tcx>,
    ctxt: &mut Context<'tcx>,
    state: &mut State,
    impl_id: DefId,
    impll: &Impl<'tcx>,
    crate_items: &CrateItems,
) {
    for impl_item_ref in impll.items {
        match impl_item_ref.kind {
            AssocItemKind::Fn { .. } => {
                let impl_item = ctxt.tcx.hir_impl_item(impl_item_ref.id);
                let ImplItem { ident, owner_id, kind, .. } = impl_item;
                let id = owner_id.to_def_id();
                let attrs = ctxt.tcx.hir_attrs(impl_item.hir_id());
                let vattrs = get_verifier_attrs(attrs, None).expect("get_verifier_attrs");
                if crate_items.is_impl_item_external(impl_item_ref.id) {
                    continue;
                }
                if vattrs.reveal_group {
                    continue;
                }
                match &kind {
                    ImplItemKind::Fn(sig, body_id) => {
                        erase_fn(
                            krate,
                            ctxt,
                            state,
                            ident.span,
                            id,
                            sig,
                            Some(impl_id),
                            false,
                            vattrs.external_body,
                            Some(body_id),
                        );
                    }
                    _ => panic!(),
                }
            }
            AssocItemKind::Type => {
                // handled in erase_trait
            }
            AssocItemKind::Const => {
                let impl_item = ctxt.tcx.hir_impl_item(impl_item_ref.id);
                let ImplItem { ident, owner_id, kind, .. } = impl_item;
                let id = owner_id.to_def_id();
                let attrs = ctxt.tcx.hir_attrs(impl_item.hir_id());
                let vattrs = get_verifier_attrs(attrs, None).expect("get_verifier_attrs");
                if crate_items.is_impl_item_external(impl_item_ref.id) {
                    continue;
                }
                match &kind {
                    ImplItemKind::Const(_, body_id) => {
                        erase_const_or_static(
                            Some(krate),
                            ctxt,
                            state,
                            ident.span,
                            id,
                            vattrs.external_body,
                            Some(body_id),
                            false,
                        );
                    }
                    _ => panic!(),
                }
            }
        }
    }
}

fn erase_datatype<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    span: Span,
    id: DefId,
    datatype: Datatype,
) {
    let datatype = Box::new(datatype);
    let path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);
    let name = state.datatype_name(&path);
    let mut lifetimes: Vec<GenericParam> = Vec::new();
    let mut typ_params: Vec<GenericParam> = Vec::new();
    let mut generic_bounds: Vec<GenericBound> = Vec::new();
    erase_mir_generics(
        ctxt,
        state,
        id,
        false,
        &mut lifetimes,
        &mut typ_params,
        &mut generic_bounds,
    );
    let generic_params = lifetimes.into_iter().chain(typ_params.into_iter()).collect();
    let span = Some(span);
    let decl = DatatypeDecl { name, span, generic_params, generic_bounds, datatype };
    state.datatype_decls.push(decl);
}

fn erase_variant_data<'tcx>(
    ctxt: &Context<'tcx>,
    state: &mut State,
    variant: &VariantDef,
) -> Fields {
    let revise_typ = |f_did: DefId, typ: Typ| {
        let attrs: Vec<_> = (if let Some(did) = f_did.as_local() {
            ctxt.tcx.hir_attrs(ctxt.tcx.local_def_id_to_hir_id(did)).iter()
        } else {
            ctxt.tcx.attrs_for_def(f_did).iter()
        })
        .cloned()
        .collect();
        let mode = get_mode(Mode::Exec, &attrs[..]);
        if mode == Mode::Spec { Box::new(TypX::Phantom(typ)) } else { typ }
    };
    match variant.ctor_kind() {
        Some(CtorKind::Fn) => {
            let mut fields: Vec<Typ> = Vec::new();
            for field in &variant.fields {
                let typ = erase_ty(ctxt, state, &ctxt.tcx.type_of(field.did).skip_binder());
                fields.push(revise_typ(field.did, typ));
            }
            Fields::Pos(fields)
        }
        None => {
            let mut fields: Vec<Field> = Vec::new();
            for field in &variant.fields {
                let ident = field.ident(ctxt.tcx);
                let name = state.field(ident.to_string());
                let typ = erase_ty(ctxt, state, &ctxt.tcx.type_of(field.did).skip_binder());
                fields.push(Field { name, typ: revise_typ(field.did, typ) });
            }
            Fields::Named(fields)
        }
        Some(CtorKind::Const) => {
            assert!(variant.fields.len() == 0);
            Fields::Pos(vec![])
        }
    }
}

// Treat external_body datatypes as abstract (erase the original fields)
fn erase_abstract_datatype<'tcx>(ctxt: &Context<'tcx>, state: &mut State, span: Span, id: DefId) {
    let mut fields: Vec<Typ> = Vec::new();
    let mir_generics = ctxt.tcx.generics_of(id);
    for gparam in mir_generics.own_params.iter() {
        // Rust requires all lifetime/type variables to be mentioned in the fields,
        // so introduce a dummy field for each lifetime/type variable
        match gparam.kind {
            GenericParamDefKind::Lifetime => {
                let x = state.lifetime((gparam.name.to_string(), Some(gparam.index)));
                fields.push(Box::new(TypX::Ref(TypX::mk_bool(), Some(x), Mutability::Not)));
            }
            GenericParamDefKind::Type { .. } => {
                let x = state.typ_param(gparam.name.to_string(), Some(gparam.index));
                fields.push(Box::new(TypX::Phantom(Box::new(TypX::TypParam(x)))));
            }
            GenericParamDefKind::Const { .. } => {
                // no dummy needed
            }
        }
    }
    let datatype = Datatype::Struct(Fields::Pos(fields));
    erase_datatype(ctxt, state, span, id, datatype);
}

fn erase_mir_datatype<'tcx>(ctxt: &Context<'tcx>, state: &mut State, id: DefId) {
    let span = ctxt.tcx.def_span(id);

    let attrs: Vec<_> = (if let Some(did) = id.as_local() {
        ctxt.tcx.hir_attrs(ctxt.tcx.local_def_id_to_hir_id(did)).iter()
    } else {
        ctxt.tcx.attrs_for_def(id).iter()
    })
    .cloned()
    .collect();

    let rust_item = verus_items::get_rust_item(ctxt.tcx, id);
    if let Some(
        RustItem::Box
        | RustItem::Rc
        | RustItem::Arc
        | RustItem::AllocGlobal
        | RustItem::ManuallyDrop
        | RustItem::PhantomData,
    ) = rust_item
    {
        return;
    }

    let vattrs = get_verifier_attrs(&attrs[..], None).expect("get_verifier_attrs");
    if vattrs.external_type_specification {
        return;
    }

    let path = def_id_to_vir_path(ctxt.tcx, &ctxt.verus_items, id);

    // Check if the struct is 'external_body'
    // (Recall that the 'external_body' label may have been applied by a proxy type,
    // so we can't check the vattrs of the datatype definition directly.
    // Need to check the VIR instead.)
    let is_external_body = match ctxt.datatypes.get(&path) {
        Some(dt) => match dt.x.transparency {
            DatatypeTransparency::Never => true,
            DatatypeTransparency::WhenVisible(_) => false,
        },
        // We may see extra datatypes from imported libraries that we
        // use for associated types:
        None => true,
    };
    if is_external_body {
        erase_abstract_datatype(ctxt, state, span, id);
        return;
    }

    let adt_def = ctxt.tcx.adt_def(id);
    if adt_def.is_struct() {
        let fields = erase_variant_data(ctxt, state, adt_def.non_enum_variant());
        let datatype = Datatype::Struct(fields);
        erase_datatype(ctxt, state, span, id, datatype);
    } else if adt_def.is_enum() {
        let mut variants: Vec<(Id, Fields)> = Vec::new();
        for variant in adt_def.variants().iter() {
            let name = state.variant(variant.ident(ctxt.tcx).to_string());
            let fields = erase_variant_data(ctxt, state, variant);
            variants.push((name, fields));
        }
        let datatype = Datatype::Enum(variants);
        erase_datatype(ctxt, state, span, id, datatype);
    } else if adt_def.is_union() {
        let fields = erase_variant_data(ctxt, state, adt_def.non_enum_variant());
        let datatype = Datatype::Union(fields);
        erase_datatype(ctxt, state, span, id, datatype);
    } else {
        panic!("unexpected datatype {:?}", id);
    }
}

pub(crate) fn gen_check_tracked_lifetimes<'tcx>(
    cmd_line_args: crate::config::Args,
    tcx: TyCtxt<'tcx>,
    verus_items: Arc<VerusItems>,
    krate: &'tcx Crate<'tcx>,
    erasure_hints: &ErasureHints,
    crate_items: &CrateItems,
) -> State {
    let mut ctxt = Context {
        _cmd_line_args: cmd_line_args,
        tcx,
        verus_items,
        types_opt: None,
        functions: HashMap::new(),
        datatypes: HashMap::new(),
        ignored_functions: HashSet::new(),
        calls: HashMap::new(),
        condition_modes: HashMap::new(),
        var_modes: HashMap::new(),
        ret_spec: None,
    };
    let mut state = State::new();
    let mut id_to_hir: HashMap<AstId, Vec<HirId>> = HashMap::new();
    for (hir_id, vir_id) in &erasure_hints.hir_vir_ids {
        if !id_to_hir.contains_key(vir_id) {
            id_to_hir.insert(*vir_id, vec![]);
        }
        id_to_hir.get_mut(vir_id).unwrap().push(*hir_id);
    }
    for f in &erasure_hints.vir_crate.functions {
        ctxt.functions.insert(f.x.name.clone(), Some(f.clone())).map(|_| panic!("{:?}", &f.x.name));
    }
    for d in &erasure_hints.vir_crate.datatypes {
        if let Dt::Path(path) = &d.x.name {
            ctxt.datatypes.insert(path.clone(), d.clone()).map(|_| panic!("{:?}", &path));
        }
    }
    for (id, _span) in &erasure_hints.ignored_functions {
        ctxt.ignored_functions.insert(*id);
    }
    for (hir_id, span, call) in &erasure_hints.resolved_calls {
        if ctxt.calls.contains_key(hir_id) {
            if &ctxt.calls[hir_id] != call {
                panic!("inconsistent resolved_calls: {:?}", span);
            }
        } else {
            ctxt.calls.insert(*hir_id, call.clone());
        }
    }
    for (span, mode) in &erasure_hints.erasure_modes.condition_modes {
        if crate::spans::from_raw_span(&span.raw_span).is_none() {
            continue;
        }
        if !id_to_hir.contains_key(&span.id) {
            dbg!(span, span.id);
            panic!("missing id_to_hir");
        }
        for hir_id in &id_to_hir[&span.id] {
            if ctxt.condition_modes.contains_key(hir_id) {
                if &ctxt.condition_modes[hir_id] != mode {
                    panic!("inconsistent condition_modes: {:?}", span);
                }
            } else {
                ctxt.condition_modes.insert(*hir_id, *mode);
            }
        }
    }
    for (span, mode) in &erasure_hints.erasure_modes.var_modes {
        if crate::spans::from_raw_span(&span.raw_span).is_none() {
            continue;
        }
        if !id_to_hir.contains_key(&span.id) {
            dbg!(span, span.id);
            panic!("missing id_to_hir");
        }
        for hir_id in &id_to_hir[&span.id] {
            if ctxt.var_modes.contains_key(hir_id) {
                if &ctxt.var_modes[hir_id] != mode {
                    panic!("inconsistent var_modes: {:?}", span);
                }
            } else {
                ctxt.var_modes.insert(*hir_id, *mode);
            }
        }
    }
    for (hir_id, mode) in &erasure_hints.direct_var_modes {
        ctxt.var_modes.insert(*hir_id, *mode).map(|v| panic!("{:?}", v));
    }
    for owner in krate.owners.iter() {
        if let MaybeOwner::Owner(owner) = owner {
            match owner.node() {
                OwnerNode::Item(item) => {
                    if !matches!(&item.kind, ItemKind::Impl(_))
                        && crate_items.is_item_external(item.item_id())
                    {
                        // item is external
                        continue;
                    }
                    match &item.kind {
                        ItemKind::Trait(
                            IsAuto::No,
                            Safety::Safe,
                            _ident,
                            _trait_generics,
                            _bounds,
                            _trait_items,
                        ) => {
                            if crate_items.is_item_external(item.item_id()) {
                                continue;
                            }
                            // We only need traits with associated type declarations.
                            // Process traits early so we can see which traits we need.
                            let id = item.owner_id.to_def_id();
                            erase_trait(&ctxt, &mut state, id);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(id) = ctxt.tcx.lang_items().copy_trait() {
        erase_trait(&ctxt, &mut state, id);
    }
    if let Some(id) = ctxt.tcx.lang_items().clone_trait() {
        erase_trait(&ctxt, &mut state, id);
    }

    for owner in krate.owners.iter() {
        if let MaybeOwner::Owner(owner) = owner {
            match owner.node() {
                OwnerNode::Item(item) => {
                    if crate_items.is_item_external(item.item_id()) {
                        // item is external
                        continue;
                    }
                    let attrs = tcx.hir_attrs(item.hir_id());
                    let vattrs = get_verifier_attrs(attrs, None).expect("get_verifier_attrs");
                    if vattrs.internal_reveal_fn || vattrs.internal_const_body {
                        continue;
                    }
                    let id = item.owner_id.to_def_id();
                    match &item.kind {
                        ItemKind::Use { .. } => {}
                        ItemKind::ExternCrate { .. } => {}
                        ItemKind::Mod { .. } => {}
                        ItemKind::ForeignMod { .. } => {}
                        ItemKind::Macro(..) => {}
                        ItemKind::TyAlias(..) => {}
                        ItemKind::GlobalAsm { .. } => {}
                        ItemKind::Struct(_ident, _s, _generics) => {
                            state.reach_datatype(&ctxt, id);
                        }
                        ItemKind::Enum(_ident, _e, _generics) => {
                            state.reach_datatype(&ctxt, id);
                        }
                        ItemKind::Union(_ident, _e, _generics) => {
                            state.reach_datatype(&ctxt, id);
                        }
                        ItemKind::Const(_ident, _ty, _, body_id)
                        | ItemKind::Static(_ident, _ty, _, body_id) => {
                            if vattrs.size_of_global || vattrs.item_broadcast_use {
                                continue;
                            }
                            erase_const_or_static(
                                Some(krate),
                                &mut ctxt,
                                &mut state,
                                item.span,
                                id,
                                vattrs.external_body,
                                Some(body_id),
                                matches!(&item.kind, ItemKind::Static(..)),
                            );
                        }
                        ItemKind::Fn { ident, sig, body: body_id, .. } => {
                            if vattrs.reveal_group {
                                continue;
                            }
                            if !vattrs.external_fn_specification {
                                erase_fn(
                                    krate,
                                    &mut ctxt,
                                    &mut state,
                                    ident.span,
                                    id,
                                    sig,
                                    None,
                                    false,
                                    vattrs.external_body,
                                    Some(body_id),
                                );
                            } else {
                                let body = ctxt.tcx.hir_body(*body_id);
                                let (def_id, _) = crate::rust_to_vir_func::get_external_def_id(
                                    ctxt.tcx,
                                    &ctxt.verus_items,
                                    id,
                                    body_id,
                                    body,
                                    sig,
                                )
                                .unwrap();

                                // Case where the external function is local - it doesn't
                                // end up in the 'imported_fun_worklist' in this case
                                if def_id.as_local().is_some() {
                                    import_fn(&mut ctxt, &mut state, def_id);
                                }
                            }
                        }
                        ItemKind::Trait(
                            IsAuto::No,
                            Safety::Safe | Safety::Unsafe,
                            _ident,
                            _trait_generics,
                            _bounds,
                            trait_items,
                        ) => {
                            let ex_trait_id_for =
                                crate::rust_to_vir_trait::external_trait_specification_of(
                                    tcx,
                                    trait_items,
                                    &vattrs,
                                )
                                .expect("already checked by rust_to_vir_trait")
                                .map(|r| r.def_id);
                            if let Some(ex_trait_id_for) = ex_trait_id_for {
                                if ex_trait_id_for.as_local().is_none() {
                                    continue;
                                }
                            }
                            erase_trait_item(
                                krate,
                                &mut ctxt,
                                &mut state,
                                id,
                                ex_trait_id_for,
                                trait_items,
                            );
                        }
                        ItemKind::Impl(impll) => {
                            if vattrs.external_trait_blanket {
                                continue;
                            }
                            erase_impl(krate, &mut ctxt, &mut state, id, impll, crate_items);
                        }
                        ItemKind::TraitAlias(_, _, _) => {
                            dbg!(item);
                            panic!("unexpected item");
                        }
                        ItemKind::Trait(IsAuto::Yes, _, _, _, _, _) => {
                            dbg!(item);
                            panic!("unexpected item");
                        }
                    }
                }
                OwnerNode::TraitItem(_trait_item) => {
                    // handled by ItemKind::Trait
                }
                OwnerNode::Crate(_mod_) => {}
                OwnerNode::ImplItem(_) => {}
                OwnerNode::ForeignItem(_foreign_item) => {}
                OwnerNode::Synthetic => {}
            }
        }
    }
    loop {
        if let Some(const_or_static) = state.const_static_worklist.pop() {
            import_const_static(
                &mut ctxt,
                &mut state,
                const_or_static.id,
                const_or_static.is_static,
            );
            continue;
        }
        if let Some(id) = state.imported_fun_worklist.pop() {
            import_fn(&mut ctxt, &mut state, id);
            continue;
        }
        if let Some(id) = state.datatype_worklist.pop() {
            erase_mir_datatype(&ctxt, &mut state, id);
            continue;
        }
        if let Some(id) = state.impl_assocs_worklist.pop() {
            erase_impl_assocs(&ctxt, &mut state, id);
            continue;
        }
        break;
    }
    assert!(state.rename_bound_for.len() == 0);
    state
}
