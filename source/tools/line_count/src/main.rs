use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    ops::RangeInclusive,
    rc::Rc,
};

use serde::Serialize;
use tabled::settings::{
    Alignment, Modify, Style,
    object::{Columns, Rows},
};
use verus_syn::{Attribute, File, Meta, MetaList, Signature, spanned::Spanned, visit::Visit};

struct Config {
    print_all: bool,
    json: bool,
    no_external_by_default: bool,
    delimiters_are_layout: bool,
    proofs_arent_trusted: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let program = args[0].clone();

    let mut opts = getopts::Options::new();
    opts.optflag("h", "help", "print this help menu");
    opts.optflag("p", "print-all", "print all the annotated files");
    opts.optflag("", "no-external-by-default", "do not ignore items outside of verus! by default");
    opts.optflag("", "json", "output as machine-readable json");
    opts.optflag("", "delimiters-are-layout", "consider delimiter-only lines as layout");
    opts.optflag("", "proofs-arent-trusted", "do not apply trusted to proofs");
    opts.optflag("", "one-file", "parse one file, isntead of using the .d file produced by rustc");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => {
            panic!("{}", f.to_string())
        }
    };

    fn print_usage(program: &str, opts: getopts::Options) {
        let brief = format!("Usage: {} DEPS_FILE.d [options]", program);
        print!("{}", opts.usage(&brief));
    }

    if matches.opt_present("h") {
        print_usage(&program, opts);
        return;
    }

    let path = if !matches.free.is_empty() {
        matches.free[0].clone()
    } else {
        print_usage(&program, opts);
        return;
    };
    let path = std::path::Path::new(&path);

    let config = Config {
        print_all: matches.opt_present("p"),
        json: matches.opt_present("json"),
        no_external_by_default: matches.opt_present("no-external-by-default"),
        delimiters_are_layout: matches.opt_present("delimiters-are-layout"),
        proofs_arent_trusted: matches.opt_present("proofs-arent-trusted"),
    };

    let run_mode_paths = if matches.opt_present("one-file") {
        RunMode::OneFile(path)
    } else {
        RunMode::DepsPath(path)
    };

    match run(config, run_mode_paths) {
        Ok(()) => (),
        Err(err) => {
            eprintln!("error: {}", err);
            std::process::exit(1);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, PartialOrd, Ord)]
enum CodeKind {
    Trusted,
    Spec,
    Proof,
    Exec,
    Directives,
    Definitions,
    Comment,
    Layout,
}

impl CodeKind {
    fn join_prefer_left(&self, other: CodeKind) -> CodeKind {
        match (self, other) {
            (CodeKind::Spec, _) => CodeKind::Spec,
            (_, CodeKind::Spec) => CodeKind::Spec,
            (CodeKind::Proof, _) => CodeKind::Proof,
            (_, CodeKind::Proof) => CodeKind::Proof,
            (CodeKind::Exec, _) => CodeKind::Exec,
            (_, CodeKind::Exec) => CodeKind::Exec,
            (other, _) => *other,
        }
    }
}

trait ToCodeKind {
    fn to_code_kind(&self) -> CodeKind;
}

impl ToCodeKind for verus_syn::DataMode {
    fn to_code_kind(&self) -> CodeKind {
        match self {
            verus_syn::DataMode::Ghost(_) => CodeKind::Spec,
            verus_syn::DataMode::Tracked(_) => CodeKind::Proof,
            verus_syn::DataMode::Exec(_) => CodeKind::Exec,
            verus_syn::DataMode::Default => CodeKind::Exec,
        }
    }
}

impl ToCodeKind for verus_syn::FnMode {
    fn to_code_kind(&self) -> CodeKind {
        match self {
            verus_syn::FnMode::Spec(_) | verus_syn::FnMode::SpecChecked(_) => CodeKind::Spec,
            // REVIEW: ProofAxiom may need to be treatead as trusted, with an explicit LineContent entry
            verus_syn::FnMode::Proof(_) | verus_syn::FnMode::ProofAxiom(_) => CodeKind::Proof,
            verus_syn::FnMode::Exec(_) | verus_syn::FnMode::Default => CodeKind::Exec,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum StateMachineCode {
    NameAndFields,
    Transition,
    Property,
    StructWithInvariantBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum LineContent {
    Const(CodeKind),
    Code(CodeKind),
    DatatypeDecl,
    TypeDefinition,
    Trait,
    ProofBlock,
    ProofDirective, // Assert, Assume, Reveal, ...
    ProofBinding,
    Impl,
    Signature(CodeKind),
    FunctionSpec,
    Body(CodeKind),
    Directive,
    MacroDefinition,
    GhostTracked(CodeKind),
    Comment,
    StateMachine(StateMachineCode),
    Atomic,
}

struct LineInfo {
    kinds: HashSet<CodeKind>,
    #[allow(dead_code)]
    path: Vec<String>,
    line_content: HashSet<LineContent>,
    text: String,
}

struct FileStats {
    lines: Box<[LineInfo]>,
}

fn to_lines_range(spanned: &impl Spanned) -> RangeInclusive<usize> {
    let span = spanned.span();
    let proc_macro2::LineColumn { line: start_line, column: _ } = span.start();
    let proc_macro2::LineColumn { line: end_line, column: _ } = span.end();
    (start_line - 1)..=(end_line - 1)
}

impl FileStats {
    #[allow(dead_code)]
    fn mark_kind(&mut self, spanned: &impl Spanned, kind: CodeKind) {
        for l in to_lines_range(spanned) {
            self.lines[l]
                .kinds
                .retain(|x| !matches!(x, CodeKind::Spec | CodeKind::Proof | CodeKind::Exec));
            self.lines[l].kinds.insert(kind);
        }
    }

    #[allow(dead_code)]
    fn mark_additional_kind(&mut self, spanned: &impl Spanned, kind: CodeKind) {
        for l in to_lines_range(spanned) {
            self.lines[l].kinds.insert(kind);
        }
    }

    fn mark_content(&mut self, spanned: &impl Spanned, content: LineContent) {
        for l in to_lines_range(spanned) {
            self.lines[l].line_content.insert(content);
        }
    }

    fn mark(&mut self, spanned: &(impl Spanned + Debug), kind: CodeKind, content: LineContent) {
        for l in to_lines_range(spanned) {
            self.lines[l]
                .kinds
                .retain(|x| !matches!(x, CodeKind::Spec | CodeKind::Proof | CodeKind::Exec));
            self.lines[l].kinds.insert(kind);
            self.lines[l].line_content.insert(content);
        }
    }

    fn mark_with_additional_kind(
        &mut self,
        spanned: &impl Spanned,
        kind: CodeKind,
        content: LineContent,
    ) {
        for l in to_lines_range(spanned) {
            self.lines[l].kinds.insert(kind);
            self.lines[l].line_content.insert(content);
        }
    }
}

struct Visitor<'f> {
    inside_verus_macro_or_verify_or_consider: u64,
    file_stats: &'f mut FileStats,
    in_body: Option<CodeKind>,
    trusted: u64,
    in_proof_directive: u64,
    in_state_machine_macro: u64,
    inside_line_count_ignore_or_external: u64,
    config: Rc<Config>,
}

impl<'f> Visitor<'f> {
    fn active(&self) -> bool {
        self.inside_line_count_ignore_or_external == 0
            && (self.inside_verus_macro_or_verify_or_consider > 0
                || self.config.no_external_by_default)
    }

    #[allow(dead_code)]
    fn mark_kind(&mut self, spanned: &impl Spanned, kind: CodeKind) {
        if self.active() {
            self.file_stats.mark_kind(spanned, kind);
        }
    }

    #[allow(dead_code)]
    fn mark_additional_kind(&mut self, spanned: &impl Spanned, kind: CodeKind) {
        if self.active() {
            self.file_stats.mark_additional_kind(spanned, kind)
        }
    }

    fn mark_content(&mut self, spanned: &impl Spanned, content: LineContent) {
        if self.active() {
            self.file_stats.mark_content(spanned, content);
        }
    }

    fn mark(&mut self, spanned: &(impl Spanned + Debug), kind: CodeKind, content: LineContent) {
        if self.active() {
            self.file_stats.mark(spanned, kind, content);
        }
    }

    fn mark_with_additional_kind(
        &mut self,
        spanned: &impl Spanned,
        kind: CodeKind,
        content: LineContent,
    ) {
        if self.active() {
            self.file_stats.mark_with_additional_kind(spanned, kind, content);
        }
    }
}

impl<'ast, 'f> verus_syn::visit::Visit<'ast> for Visitor<'f> {
    fn visit_assert(&mut self, i: &'ast verus_syn::Assert) {
        self.in_proof_directive += 1;
        self.mark(i, self.mode_or_trusted(CodeKind::Proof), LineContent::ProofDirective);
        verus_syn::visit::visit_assert(self, i);
        self.in_proof_directive -= 1;
    }

    fn visit_assert_forall(&mut self, i: &'ast verus_syn::AssertForall) {
        self.in_proof_directive += 1;
        self.mark(i, self.mode_or_trusted(CodeKind::Proof), LineContent::ProofDirective);
        verus_syn::visit::visit_assert_forall(self, i);
        self.in_proof_directive -= 1;
    }

    fn visit_assume(&mut self, i: &'ast verus_syn::Assume) {
        self.in_proof_directive += 1;
        self.mark(i, self.mode_or_trusted(CodeKind::Proof), LineContent::ProofDirective);
        verus_syn::visit::visit_assume(self, i);
        self.in_proof_directive -= 1;
    }

    #[allow(unreachable_code)]
    fn visit_data(&mut self, _i: &'ast verus_syn::Data) {
        panic!("data unsupported");
        verus_syn::visit::visit_data(self, _i);
    }

    fn visit_decreases(&mut self, i: &'ast verus_syn::Decreases) {
        // self.mark(i, self.mode_or_trusted(CodeKind::Spec), LineContent::FunctionSpec);
        verus_syn::visit::visit_decreases(self, i);
    }

    fn visit_ensures(&mut self, i: &'ast verus_syn::Ensures) {
        // self.mark(i, self.mode_or_trusted(CodeKind::Spec), LineContent::FunctionSpec);
        verus_syn::visit::visit_ensures(self, i);
    }

    fn visit_block(&mut self, i: &'ast verus_syn::Block) {
        if let Some(content_code_kind) = self.in_body {
            if self.in_proof_directive == 0 {
                self.mark(
                    &i,
                    self.mode_or_trusted(content_code_kind),
                    LineContent::Code(content_code_kind),
                )
            }
        }
        verus_syn::visit::visit_block(self, i);
    }

    fn visit_expr(&mut self, i: &'ast verus_syn::Expr) {
        if let Some(content_code_kind) = self.in_body {
            if self.in_proof_directive == 0 {
                self.mark(
                    &i,
                    self.mode_or_trusted(content_code_kind),
                    LineContent::Code(content_code_kind),
                )
            }
        }
        let entered_proof_directive = match i {
            verus_syn::Expr::Unary(verus_syn::ExprUnary {
                op: verus_syn::UnOp::Proof(..),
                attrs: _,
                expr,
            }) => {
                self.mark(expr, self.mode_or_trusted(CodeKind::Proof), LineContent::ProofBlock);
                self.in_proof_directive += 1;
                true
            }
            _ => false,
        };
        verus_syn::visit::visit_expr(self, i);
        if entered_proof_directive {
            self.in_proof_directive -= 1;
        }
    }

    fn visit_expr_block(&mut self, i: &'ast verus_syn::ExprBlock) {
        verus_syn::visit::visit_expr_block(self, i);
    }

    fn visit_expr_call(&mut self, i: &'ast verus_syn::ExprCall) {
        // Ghost / Tracked ?
        if let verus_syn::Expr::Path(path) = &*i.func {
            if let Some(wrapper_code_kind) = (path.path.segments.len() == 1)
                .then(|| path.path.segments[0].ident.to_string())
                .and_then(|c| match c.as_str() {
                    "Ghost" => {
                        if self.in_body == Some(CodeKind::Spec) {
                            Some(self.mode_or_trusted(CodeKind::Spec))
                        } else {
                            Some(self.mode_or_trusted(CodeKind::Proof))
                        }
                    }
                    "Tracked" => Some(self.mode_or_trusted(CodeKind::Proof)),
                    _ => None,
                })
            {
                self.mark_with_additional_kind(
                    i,
                    wrapper_code_kind,
                    LineContent::GhostTracked(wrapper_code_kind),
                );
                return;
            }
        }
        verus_syn::visit::visit_expr_call(self, i);
    }

    fn visit_expr_closure(&mut self, i: &'ast verus_syn::ExprClosure) {
        // TODO
        verus_syn::visit::visit_expr_closure(self, i);
    }

    fn visit_expr_loop(&mut self, i: &'ast verus_syn::ExprLoop) {
        if let Some(decreases) = &i.decreases {
            self.mark(
                decreases,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(invariant_except_break) = &i.invariant_except_break {
            self.mark(
                &invariant_except_break,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(invariant) = &i.invariant {
            self.mark(
                &invariant,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(invariant_ensures) = &i.invariant_ensures {
            self.mark(
                &invariant_ensures,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(ensures) = &i.ensures {
            self.mark(&ensures, self.mode_or_trusted(CodeKind::Proof), LineContent::ProofDirective);
        }
        self.visit_block(&i.body);
    }

    fn visit_expr_while(&mut self, i: &'ast verus_syn::ExprWhile) {
        if let Some(decreases) = &i.decreases {
            self.mark(
                decreases,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(invariant_except_break) = &i.invariant_except_break {
            self.mark(
                &invariant_except_break,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(invariant) = &i.invariant {
            self.mark(
                &invariant,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(invariant_ensures) = &i.invariant_ensures {
            self.mark(
                &invariant_ensures,
                self.mode_or_trusted(CodeKind::Proof),
                LineContent::ProofDirective,
            );
        }
        if let Some(ensures) = &i.ensures {
            self.mark(&ensures, self.mode_or_trusted(CodeKind::Proof), LineContent::ProofDirective);
        }
        self.visit_expr(&i.cond);
        self.visit_block(&i.body);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast verus_syn::ImplItemFn) {
        let content_code_kind = i.sig.mode.to_code_kind();
        let exit = self.item_attr_enter(&i.attrs);
        let code_kind = self.mode_or_trusted(content_code_kind);
        // self.mark(&i.block, code_kind, LineContent::Code(content_code_kind));
        self.mark_content(&i.block, LineContent::Body(content_code_kind));
        self.handle_signature(content_code_kind, code_kind, &i.sig);
        self.in_body = Some(content_code_kind);
        self.visit_block(&i.block);
        self.in_body = None;
        exit.exit(self);
    }

    fn visit_item(&mut self, i: &'ast verus_syn::Item) {
        match i {
            verus_syn::Item::Impl(_) => {
                self.mark_content(i, LineContent::Impl);
            }
            _ => (),
        }
        verus_syn::visit::visit_item(self, i);
    }

    fn visit_item_const(&mut self, i: &'ast verus_syn::ItemConst) {
        let exit = self.item_attr_enter(&i.attrs);
        self.mark(
            i,
            self.mode_or_trusted(i.mode.to_code_kind()),
            LineContent::Const(i.mode.to_code_kind()),
        );
        verus_syn::visit::visit_item_const(self, i);
        exit.exit(self);
    }

    fn visit_item_enum(&mut self, i: &'ast verus_syn::ItemEnum) {
        let exit = self.item_attr_enter(&i.attrs);
        self.mark(&i, self.mode_or_trusted(i.mode.to_code_kind()), LineContent::DatatypeDecl);
        verus_syn::visit::visit_item_enum(self, i);
        exit.exit(self);
    }

    fn visit_item_extern_crate(&mut self, i: &'ast verus_syn::ItemExternCrate) {
        let exit = self.item_attr_enter(&i.attrs);
        verus_syn::visit::visit_item_extern_crate(self, i);
        exit.exit(self);
    }

    fn visit_item_fn(&mut self, i: &'ast verus_syn::ItemFn) {
        let exit = self.item_attr_enter(&i.attrs);
        let content_code_kind = self.fn_code_kind(i.sig.mode.to_code_kind());
        let code_kind = self.mode_or_trusted(content_code_kind);
        // self.mark(&i.block, code_kind, LineContent::Code(content_code_kind));
        self.mark_content(&i.block, LineContent::Body(content_code_kind));
        self.handle_signature(content_code_kind, code_kind, &i.sig);
        self.in_body = Some(content_code_kind);
        self.visit_block(&i.block);
        self.in_body = None;
        exit.exit(self);
    }

    fn visit_item_foreign_mod(&mut self, i: &'ast verus_syn::ItemForeignMod) {
        let exit = self.item_attr_enter(&i.attrs);
        verus_syn::visit::visit_item_foreign_mod(self, i);
        exit.exit(self);
    }

    fn visit_item_impl(&mut self, i: &'ast verus_syn::ItemImpl) {
        let exit = self.item_attr_enter(&i.attrs);
        verus_syn::visit::visit_item_impl(self, i);
        exit.exit(self);
    }

    fn visit_item_macro(&mut self, i: &'ast verus_syn::ItemMacro) {
        verus_syn::visit::visit_item_macro(self, i);
    }

    fn visit_item_mod(&mut self, i: &'ast verus_syn::ItemMod) {
        let exit = self.item_attr_enter(&i.attrs);
        if i.content.is_none() {
            self.mark(&i, CodeKind::Directives, LineContent::Directive);
        }
        verus_syn::visit::visit_item_mod(self, i);
        exit.exit(self);
    }

    fn visit_item_static(&mut self, i: &'ast verus_syn::ItemStatic) {
        let exit = self.item_attr_enter(&i.attrs);
        verus_syn::visit::visit_item_static(self, i);
        exit.exit(self);
    }

    fn visit_item_struct(&mut self, i: &'ast verus_syn::ItemStruct) {
        let exit = self.item_attr_enter(&i.attrs);
        self.mark(&i, self.mode_or_trusted(i.mode.to_code_kind()), LineContent::DatatypeDecl);
        verus_syn::visit::visit_item_struct(self, i);
        exit.exit(self);
    }

    fn visit_item_trait(&mut self, i: &'ast verus_syn::ItemTrait) {
        let exit = self.item_attr_enter(&i.attrs);
        self.mark_content(&i, LineContent::Trait);
        if self.trusted > 0 {
            self.mark_kind(&i, CodeKind::Trusted);
        }
        verus_syn::visit::visit_item_trait(self, i);
        exit.exit(self);
    }

    fn visit_field(&mut self, i: &'ast verus_syn::Field) {
        if let verus_syn::Type::Path(path) = &i.ty {
            if let Some(wrapper_code_kind) = (path.path.segments.len() == 1)
                .then(|| path.path.segments[0].ident.to_string())
                .and_then(|c| match c.as_str() {
                    "Ghost" => {
                        if self.in_body == Some(CodeKind::Spec) {
                            Some(self.mode_or_trusted(CodeKind::Spec))
                        } else {
                            Some(self.mode_or_trusted(CodeKind::Proof))
                        }
                    }
                    "Tracked" => Some(self.mode_or_trusted(CodeKind::Proof)),
                    _ => None,
                })
            {
                self.mark(i, wrapper_code_kind, LineContent::GhostTracked(wrapper_code_kind));
                return;
            }
        }
        verus_syn::visit::visit_field(self, i);
    }

    fn visit_item_trait_alias(&mut self, i: &'ast verus_syn::ItemTraitAlias) {
        let exit = self.item_attr_enter(&i.attrs);
        verus_syn::visit::visit_item_trait_alias(self, i);
        exit.exit(self);
    }

    fn visit_item_type(&mut self, i: &'ast verus_syn::ItemType) {
        let exit = self.item_attr_enter(&i.attrs);
        verus_syn::visit::visit_item_type(self, i);
        exit.exit(self);
    }

    fn visit_item_use(&mut self, i: &'ast verus_syn::ItemUse) {
        let exit = self.item_attr_enter(&i.attrs);
        verus_syn::visit::visit_item_use(self, i);
        exit.exit(self);
    }

    fn visit_label(&mut self, i: &'ast verus_syn::Label) {
        verus_syn::visit::visit_label(self, i);
    }

    fn visit_lifetime(&mut self, i: &'ast verus_syn::Lifetime) {
        verus_syn::visit::visit_lifetime(self, i);
    }

    fn visit_lit(&mut self, i: &'ast verus_syn::Lit) {
        verus_syn::visit::visit_lit(self, i);
    }

    fn visit_lit_bool(&mut self, i: &'ast verus_syn::LitBool) {
        verus_syn::visit::visit_lit_bool(self, i);
    }

    fn visit_lit_byte(&mut self, i: &'ast verus_syn::LitByte) {
        verus_syn::visit::visit_lit_byte(self, i);
    }

    fn visit_lit_byte_str(&mut self, i: &'ast verus_syn::LitByteStr) {
        verus_syn::visit::visit_lit_byte_str(self, i);
    }

    fn visit_lit_char(&mut self, i: &'ast verus_syn::LitChar) {
        verus_syn::visit::visit_lit_char(self, i);
    }

    fn visit_lit_float(&mut self, i: &'ast verus_syn::LitFloat) {
        verus_syn::visit::visit_lit_float(self, i);
    }

    fn visit_lit_int(&mut self, i: &'ast verus_syn::LitInt) {
        verus_syn::visit::visit_lit_int(self, i);
    }

    fn visit_lit_str(&mut self, i: &'ast verus_syn::LitStr) {
        verus_syn::visit::visit_lit_str(self, i);
    }

    fn visit_local(&mut self, i: &'ast verus_syn::Local) {
        if i.ghost.is_some() || i.tracked.is_some() {
            self.mark(i, self.mode_or_trusted(CodeKind::Proof), LineContent::ProofBinding);
        }
        verus_syn::visit::visit_local(self, i);
    }

    fn visit_macro(&mut self, i: &'ast verus_syn::Macro) {
        let mut entered_state_machine_macro = false;
        let mut entered_struct_with_invariants = false;
        let outer_last_segment = i.path.segments.last().map(|s| s.ident.to_string());
        if outer_last_segment == Some("macro_rules".into()) {
            self.mark(i, self.mode_or_trusted(CodeKind::Definitions), LineContent::MacroDefinition);
        } else if outer_last_segment == Some("verus".into()) {
            let source_toks = &i.tokens;
            let macro_content: File = verus_syn::parse2(source_toks.clone())
                .map_err(|e| {
                    dbg!(&e.span().start(), &e.span().end());
                    format!("failed to parse file macro contents: {} {:?}", e, e.span())
                })
                .expect("unexpected verus! macro content");
            self.inside_verus_macro_or_verify_or_consider += 1;
            self.visit_file(&macro_content);
            self.inside_verus_macro_or_verify_or_consider -= 1;
        } else if outer_last_segment == Some("tokenized_state_machine".into())
            || outer_last_segment == Some("state_machine".into())
        {
            // self.mark(
            //     i,
            //     self.mode_or_trusted(CodeKind::Spec),
            //     LineContent::StateMachine(StateMachineCode::NameAndFields),
            // );
            entered_state_machine_macro = true;
            self.inside_verus_macro_or_verify_or_consider += 1;
            self.in_state_machine_macro += 1;
            use proc_macro2::TokenTree;
            for tok in i.tokens.clone() {
                match tok {
                    TokenTree::Group(g) => {
                        let mut g_stream = g.stream().into_iter().peekable();
                        if !(g.delimiter() == proc_macro2::Delimiter::Brace
                            && g_stream.next().map(|t| t.to_string()) == Some("fields".into()))
                        {
                            continue;
                        }
                        if let Some(fields_g) = g_stream.next() {
                            if let TokenTree::Group(g) = fields_g {
                                self.mark(
                                    &g,
                                    self.mode_or_trusted(CodeKind::Spec),
                                    LineContent::StateMachine(StateMachineCode::NameAndFields),
                                );
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        }
                        // let mut next_t = g_stream.next();
                        let content_as_file: Option<verus_syn::File> =
                            verus_syn::parse2(proc_macro2::TokenStream::from_iter(g_stream)).ok();
                        if let Some(content_as_file) = content_as_file {
                            // self.visit_file(&content_as_file);
                            for item in content_as_file.items {
                                match item {
                                    verus_syn::Item::Macro(m) => {
                                        let last_segment =
                                            m.mac.path.segments.last().map(|s| s.ident.to_string());
                                        if last_segment == Some("transition".into())
                                            || last_segment == Some("init".into())
                                        {
                                            self.mark(
                                                &m,
                                                self.mode_or_trusted(CodeKind::Spec),
                                                LineContent::StateMachine(
                                                    StateMachineCode::Transition,
                                                ),
                                            );
                                        } else if last_segment == Some("property".into()) {
                                            self.mark(
                                                &m,
                                                self.mode_or_trusted(CodeKind::Spec),
                                                LineContent::StateMachine(
                                                    StateMachineCode::Property,
                                                ),
                                            );
                                        }
                                    }
                                    _ => self.visit_item(&item),
                                }
                            }
                        }
                    }
                    TokenTree::Ident(ident) => {
                        self.mark(
                            &ident,
                            self.mode_or_trusted(CodeKind::Spec),
                            LineContent::StateMachine(StateMachineCode::NameAndFields),
                        );
                    }
                    TokenTree::Punct(_) => (),
                    TokenTree::Literal(_) => (),
                }
            }
        } else if outer_last_segment == Some("struct_with_invariants".into()) {
            entered_struct_with_invariants = true;
            self.inside_verus_macro_or_verify_or_consider += 1;

            let mut found_braced_group = false;
            let mut tokens_here = i.tokens.clone().into_iter();
            let s = proc_macro2::TokenStream::from_iter(tokens_here.by_ref().take_while(|t| {
                match t {
                    proc_macro2::TokenTree::Group(g) => {
                        if g.delimiter() == proc_macro2::Delimiter::Brace {
                            found_braced_group = true;
                            return true;
                        }
                    }
                    _ => (),
                }
                !found_braced_group
            }));
            let content_as_file: Option<verus_syn::File> = verus_syn::parse2(s).ok();
            if let Some(content_as_file) = content_as_file {
                for item in content_as_file.items {
                    self.visit_item(&item);
                }
            }
            for tok in tokens_here {
                self.mark(
                    &tok.span(),
                    CodeKind::Spec,
                    LineContent::StateMachine(StateMachineCode::StructWithInvariantBody),
                );
            }
        } else if outer_last_segment == Some("atomic_with_ghost".into())
            || outer_last_segment == Some("my_atomic_with_ghost".into())
        // for mem allocator
        {
            let mut tokens_here = i.tokens.clone().into_iter();
            for tok in proc_macro2::TokenStream::from_iter(
                tokens_here.by_ref().take_while(|t| t.to_string() != ";"),
            ) {
                self.mark(&tok.span(), CodeKind::Exec, LineContent::Atomic);
            }
            for tok in tokens_here {
                self.mark(&tok.span(), CodeKind::Proof, LineContent::Atomic);
            }
        } else if outer_last_segment == Some("tld_get_mut".into())
            || outer_last_segment == Some("page_get_mut_inner".into())
            || outer_last_segment == Some("unused_page_get_mut_prev".into())
            || outer_last_segment == Some("unused_page_get_mut_inner".into())
            || outer_last_segment == Some("unused_page_get_mut_next".into())
            || outer_last_segment == Some("unused_page_get_mut_count".into())
            || outer_last_segment == Some("unused_page_get_mut".into())
            || outer_last_segment == Some("used_page_get_mut_prev".into())
            || outer_last_segment == Some("heap_get_pages".into())
            || outer_last_segment == Some("heap_get_pages_free_direct".into())
            || outer_last_segment == Some("used_page_get_mut_next".into())
            || outer_last_segment == Some("segment_get_mut_main".into())
            || outer_last_segment == Some("segment_get_mut_main2".into())
            || outer_last_segment == Some("segment_get_mut_local".into())
        {
            for tok in i.tokens.clone().into_iter() {
                match tok.clone() {
                    proc_macro2::TokenTree::Group(g) => {
                        if g.delimiter() == proc_macro2::Delimiter::Brace {
                            let content_as_block: Option<verus_syn::Block> =
                                verus_syn::parse2(tok.into()).ok();
                            if let Some(content_as_block) = content_as_block {
                                self.visit_block(&content_as_block);
                            }
                        }
                    }
                    _ => (),
                }
            }
        }
        verus_syn::visit::visit_macro(self, i);
        if entered_state_machine_macro {
            self.in_state_machine_macro -= 1;
            self.inside_verus_macro_or_verify_or_consider -= 1;
        }
        if entered_struct_with_invariants {
            self.inside_verus_macro_or_verify_or_consider -= 1;
        }
    }

    fn visit_macro_delimiter(&mut self, i: &'ast verus_syn::MacroDelimiter) {
        verus_syn::visit::visit_macro_delimiter(self, i);
    }

    fn visit_member(&mut self, i: &'ast verus_syn::Member) {
        verus_syn::visit::visit_member(self, i);
    }

    fn visit_meta(&mut self, i: &'ast verus_syn::Meta) {
        verus_syn::visit::visit_meta(self, i);
    }

    fn visit_meta_list(&mut self, i: &'ast verus_syn::MetaList) {
        verus_syn::visit::visit_meta_list(self, i);
    }

    fn visit_meta_name_value(&mut self, i: &'ast verus_syn::MetaNameValue) {
        verus_syn::visit::visit_meta_name_value(self, i);
    }

    fn visit_mode(&mut self, i: &'ast verus_syn::Mode) {
        verus_syn::visit::visit_mode(self, i);
    }

    fn visit_mode_exec(&mut self, i: &'ast verus_syn::ModeExec) {
        verus_syn::visit::visit_mode_exec(self, i);
    }

    fn visit_mode_ghost(&mut self, i: &'ast verus_syn::ModeGhost) {
        verus_syn::visit::visit_mode_ghost(self, i);
    }

    fn visit_mode_proof(&mut self, i: &'ast verus_syn::ModeProof) {
        verus_syn::visit::visit_mode_proof(self, i);
    }

    fn visit_mode_spec(&mut self, i: &'ast verus_syn::ModeSpec) {
        verus_syn::visit::visit_mode_spec(self, i);
    }

    fn visit_mode_spec_checked(&mut self, i: &'ast verus_syn::ModeSpecChecked) {
        verus_syn::visit::visit_mode_spec_checked(self, i);
    }

    fn visit_mode_tracked(&mut self, i: &'ast verus_syn::ModeTracked) {
        verus_syn::visit::visit_mode_tracked(self, i);
    }

    fn visit_open(&mut self, i: &'ast verus_syn::Open) {
        verus_syn::visit::visit_open(self, i);
    }

    fn visit_open_restricted(&mut self, i: &'ast verus_syn::OpenRestricted) {
        verus_syn::visit::visit_open_restricted(self, i);
    }

    fn visit_parenthesized_generic_arguments(
        &mut self,
        i: &'ast verus_syn::ParenthesizedGenericArguments,
    ) {
        verus_syn::visit::visit_parenthesized_generic_arguments(self, i);
    }

    fn visit_pat(&mut self, i: &'ast verus_syn::Pat) {
        verus_syn::visit::visit_pat(self, i);
    }

    fn visit_pat_ident(&mut self, i: &'ast verus_syn::PatIdent) {
        verus_syn::visit::visit_pat_ident(self, i);
    }

    fn visit_pat_or(&mut self, i: &'ast verus_syn::PatOr) {
        verus_syn::visit::visit_pat_or(self, i);
    }

    fn visit_pat_slice(&mut self, i: &'ast verus_syn::PatSlice) {
        verus_syn::visit::visit_pat_slice(self, i);
    }

    fn visit_pat_struct(&mut self, i: &'ast verus_syn::PatStruct) {
        verus_syn::visit::visit_pat_struct(self, i);
    }

    fn visit_pat_tuple(&mut self, i: &'ast verus_syn::PatTuple) {
        verus_syn::visit::visit_pat_tuple(self, i);
    }

    fn visit_pat_tuple_struct(&mut self, i: &'ast verus_syn::PatTupleStruct) {
        verus_syn::visit::visit_pat_tuple_struct(self, i);
    }

    fn visit_pat_type(&mut self, i: &'ast verus_syn::PatType) {
        verus_syn::visit::visit_pat_type(self, i);
    }

    fn visit_pat_wild(&mut self, i: &'ast verus_syn::PatWild) {
        verus_syn::visit::visit_pat_wild(self, i);
    }

    fn visit_path(&mut self, i: &'ast verus_syn::Path) {
        verus_syn::visit::visit_path(self, i);
    }

    fn visit_path_arguments(&mut self, i: &'ast verus_syn::PathArguments) {
        verus_syn::visit::visit_path_arguments(self, i);
    }

    fn visit_path_segment(&mut self, i: &'ast verus_syn::PathSegment) {
        verus_syn::visit::visit_path_segment(self, i);
    }

    fn visit_predicate_lifetime(&mut self, i: &'ast verus_syn::PredicateLifetime) {
        verus_syn::visit::visit_predicate_lifetime(self, i);
    }

    fn visit_predicate_type(&mut self, i: &'ast verus_syn::PredicateType) {
        verus_syn::visit::visit_predicate_type(self, i);
    }

    fn visit_publish(&mut self, i: &'ast verus_syn::Publish) {
        verus_syn::visit::visit_publish(self, i);
    }

    fn visit_qself(&mut self, i: &'ast verus_syn::QSelf) {
        verus_syn::visit::visit_qself(self, i);
    }

    fn visit_range_limits(&mut self, i: &'ast verus_syn::RangeLimits) {
        verus_syn::visit::visit_range_limits(self, i);
    }

    fn visit_receiver(&mut self, i: &'ast verus_syn::Receiver) {
        verus_syn::visit::visit_receiver(self, i);
    }

    fn visit_recommends(&mut self, i: &'ast verus_syn::Recommends) {
        // self.mark(i, self.mode_or_trusted(CodeKind::Spec), LineContent::FunctionSpec);
        verus_syn::visit::visit_recommends(self, i);
    }

    fn visit_requires(&mut self, i: &'ast verus_syn::Requires) {
        // self.mark(i, self.mode_or_trusted(CodeKind::Spec), LineContent::FunctionSpec);
        verus_syn::visit::visit_requires(self, i);
    }

    fn visit_return_type(&mut self, i: &'ast verus_syn::ReturnType) {
        verus_syn::visit::visit_return_type(self, i);
    }

    fn visit_reveal_hide(&mut self, i: &'ast verus_syn::RevealHide) {
        verus_syn::visit::visit_reveal_hide(self, i);
    }

    fn visit_signature(&mut self, i: &'ast verus_syn::Signature) {
        verus_syn::visit::visit_signature(self, i);
    }

    fn visit_signature_decreases(&mut self, i: &'ast verus_syn::SignatureDecreases) {
        verus_syn::visit::visit_signature_decreases(self, i);
    }

    fn visit_signature_invariants(&mut self, i: &'ast verus_syn::SignatureInvariants) {
        verus_syn::visit::visit_signature_invariants(self, i);
    }

    fn visit_span(&mut self, i: &proc_macro2::Span) {
        verus_syn::visit::visit_span(self, i);
    }

    fn visit_specification(&mut self, i: &'ast verus_syn::Specification) {
        verus_syn::visit::visit_specification(self, i);
    }

    fn visit_stmt(&mut self, i: &'ast verus_syn::Stmt) {
        match i {
            verus_syn::Stmt::Local(verus_syn::Local {
                attrs: _,
                let_token: _,
                tracked,
                ghost,
                pat: _,
                init,
                semi_token: _,
            }) => {
                if tracked.is_some() {
                    self.mark(i, CodeKind::Proof, LineContent::GhostTracked(CodeKind::Proof));
                    return;
                }
                if ghost.is_some() {
                    if self.in_body == Some(CodeKind::Spec) {
                        self.mark(i, CodeKind::Spec, LineContent::GhostTracked(CodeKind::Spec));
                    } else {
                        self.mark(i, CodeKind::Proof, LineContent::GhostTracked(CodeKind::Proof));
                    }
                    return;
                }
                // let a = Ghost(...)
                if let Some(right) = init {
                    if right.diverge.is_some() {
                        // the else branch of let
                        warn("else branch in let currently not supported");
                    }
                    match &*right.expr {
                        verus_syn::Expr::Call(call_expr) => {
                            let verus_syn::ExprCall { attrs: _, func, paren_token: _, args: _ } =
                                &*call_expr;
                            if let verus_syn::Expr::Path(path) = &**func {
                                if let Some(wrapper_code_kind) = (path.path.segments.len() == 1)
                                    .then(|| path.path.segments[0].ident.to_string())
                                    .and_then(|c| match c.as_str() {
                                        "Ghost" => {
                                            if self.in_body == Some(CodeKind::Spec) {
                                                Some(self.mode_or_trusted(CodeKind::Spec))
                                            } else {
                                                Some(self.mode_or_trusted(CodeKind::Proof))
                                            }
                                        }
                                        "Tracked" => Some(self.mode_or_trusted(CodeKind::Proof)),
                                        _ => None,
                                    })
                                {
                                    self.mark(
                                        i,
                                        wrapper_code_kind,
                                        LineContent::GhostTracked(wrapper_code_kind),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => (),
                    }
                }
            }
            _ => (),
        }
        verus_syn::visit::visit_stmt(self, i);
    }

    fn visit_trait_bound(&mut self, i: &'ast verus_syn::TraitBound) {
        verus_syn::visit::visit_trait_bound(self, i);
    }

    fn visit_trait_bound_modifier(&mut self, i: &'ast verus_syn::TraitBoundModifier) {
        verus_syn::visit::visit_trait_bound_modifier(self, i);
    }

    fn visit_trait_item(&mut self, i: &'ast verus_syn::TraitItem) {
        verus_syn::visit::visit_trait_item(self, i);
    }

    fn visit_trait_item_const(&mut self, i: &'ast verus_syn::TraitItemConst) {
        verus_syn::visit::visit_trait_item_const(self, i);
    }

    fn visit_trait_item_macro(&mut self, i: &'ast verus_syn::TraitItemMacro) {
        verus_syn::visit::visit_trait_item_macro(self, i);
    }

    fn visit_trait_item_fn(&mut self, i: &'ast verus_syn::TraitItemFn) {
        let exit = self.item_attr_enter(&i.attrs);
        let content_code_kind = i.sig.mode.to_code_kind();
        let code_kind = self.mode_or_trusted(content_code_kind);
        self.mark_content(&i, LineContent::Trait);
        // self.mark(&i.default, code_kind, LineContent::Code(content_code_kind));
        self.mark_content(&i.default, LineContent::Body(content_code_kind));
        self.handle_signature(content_code_kind, code_kind, &i.sig);
        self.in_body = Some(content_code_kind);
        if let Some(default) = &i.default {
            self.visit_block(default);
        }
        verus_syn::visit::visit_trait_item_fn(self, i);
        self.in_body = None;
        exit.exit(self);
    }

    fn visit_trait_item_type(&mut self, i: &'ast verus_syn::TraitItemType) {
        self.mark(&i, CodeKind::Definitions, LineContent::TypeDefinition);
        verus_syn::visit::visit_trait_item_type(self, i);
    }

    fn visit_type(&mut self, i: &'ast verus_syn::Type) {
        // self.mark(&i, CodeKind::Definitions, LineContent::TypeDefinition);
        verus_syn::visit::visit_type(self, i);
    }

    fn visit_use_tree(&mut self, i: &'ast verus_syn::UseTree) {
        self.mark(i, CodeKind::Directives, LineContent::Directive);
        verus_syn::visit::visit_use_tree(self, i);
    }

    fn visit_view(&mut self, i: &'ast verus_syn::View) {
        verus_syn::visit::visit_view(self, i);
    }

    fn visit_where_clause(&mut self, i: &'ast verus_syn::WhereClause) {
        verus_syn::visit::visit_where_clause(self, i);
    }

    fn visit_where_predicate(&mut self, i: &'ast verus_syn::WherePredicate) {
        verus_syn::visit::visit_where_predicate(self, i);
    }
}

struct ItemAttrExit {
    entered_trusted: bool,
    entered_ignore: bool,
    entered_verify: bool,
    entered_external: bool,
    entered_consider: bool,
}

impl ItemAttrExit {
    fn exit(self, visitor: &mut Visitor) {
        if self.entered_trusted {
            visitor.trusted -= 1;
        }
        if self.entered_ignore {
            visitor.inside_line_count_ignore_or_external -= 1;
        }
        if self.entered_verify {
            visitor.inside_verus_macro_or_verify_or_consider -= 1;
        }
        if self.entered_external {
            visitor.inside_line_count_ignore_or_external -= 1;
        }
        if self.entered_consider {
            visitor.inside_verus_macro_or_verify_or_consider -= 1;
        }
    }
}

impl<'f> Visitor<'f> {
    fn item_attr_enter(&mut self, attrs: &Vec<Attribute>) -> ItemAttrExit {
        for attr in attrs.iter() {
            if let Meta::Path(path) = &attr.meta {
                let mut path_iter = path.segments.iter();
                match (path_iter.next(), path_iter.next(), path_iter.next()) {
                    (Some(first), Some(second), None)
                        if first.ident == "verus" && second.ident == "trusted" =>
                    {
                        self.trusted += 1;
                        return ItemAttrExit {
                            entered_trusted: true,
                            entered_ignore: false,
                            entered_verify: false,
                            entered_external: false,
                            entered_consider: false,
                        };
                    }
                    (Some(first), Some(second), Some(third))
                        if first.ident == "verus"
                            && second.ident == "line_count"
                            && third.ident == "ignore" =>
                    {
                        self.inside_line_count_ignore_or_external += 1;
                        return ItemAttrExit {
                            entered_trusted: false,
                            entered_ignore: true,
                            entered_verify: false,
                            entered_external: false,
                            entered_consider: false,
                        };
                    }
                    (Some(first), Some(second), Some(third))
                        if first.ident == "verus"
                            && second.ident == "line_count"
                            && third.ident == "consider" =>
                    {
                        self.inside_verus_macro_or_verify_or_consider += 1;
                        return ItemAttrExit {
                            entered_trusted: false,
                            entered_ignore: false,
                            entered_verify: false,
                            entered_external: false,
                            entered_consider: true,
                        };
                    }
                    (Some(first), Some(second), None)
                        if first.ident == "verifier" && second.ident == "verify" =>
                    {
                        self.inside_verus_macro_or_verify_or_consider += 1;
                        return ItemAttrExit {
                            entered_trusted: false,
                            entered_ignore: false,
                            entered_verify: true,
                            entered_external: false,
                            entered_consider: false,
                        };
                    }
                    (Some(first), Some(second), None)
                        if first.ident == "verifier" && second.ident == "external" =>
                    {
                        self.inside_line_count_ignore_or_external += 1;
                        return ItemAttrExit {
                            entered_trusted: false,
                            entered_ignore: false,
                            entered_verify: false,
                            entered_external: true,
                            entered_consider: false,
                        };
                    }
                    _ => {}
                }
            }

            if attr.path().segments.first().map(|x| x.ident == "doc").unwrap_or(false) {
            } else {
                self.mark(
                    &attr,
                    self.mode_or_trusted(CodeKind::Directives),
                    LineContent::Directive,
                );
            }
        }
        ItemAttrExit {
            entered_trusted: false,
            entered_ignore: false,
            entered_verify: false,
            entered_external: false,
            entered_consider: false,
        }
    }

    fn fn_code_kind(&self, kind: CodeKind) -> CodeKind {
        if self.in_state_machine_macro > 0 { kind.join_prefer_left(CodeKind::Spec) } else { kind }
    }

    fn mode_or_trusted(&self, kind: CodeKind) -> CodeKind {
        if self.trusted > 0 { CodeKind::Trusted } else { kind }
    }

    fn handle_signature(
        &mut self,
        content_code_kind: CodeKind,
        code_kind: CodeKind,
        sig: &Signature,
    ) {
        self.mark(&sig, code_kind, LineContent::Signature(content_code_kind));
        if code_kind != CodeKind::Spec {
            if let Some(requires) = &sig.spec.requires {
                self.mark(
                    requires,
                    self.mode_or_trusted(CodeKind::Spec),
                    LineContent::FunctionSpec,
                );
            }
            if let Some(ensures) = &sig.spec.ensures {
                self.mark(ensures, self.mode_or_trusted(CodeKind::Spec), LineContent::FunctionSpec);
            }
            if let Some(decreases) = &sig.spec.decreases {
                self.mark(
                    decreases,
                    self.mode_or_trusted(CodeKind::Spec),
                    LineContent::FunctionSpec,
                );
            }
        }
        for p in &sig.inputs {
            match &p.kind {
                verus_syn::FnArgKind::Receiver(_) => (),
                verus_syn::FnArgKind::Typed(pt) => {
                    if let verus_syn::Type::Path(path) = &*pt.ty {
                        if let Some(wrapper_code_kind) = (path.path.segments.len() == 1)
                            .then(|| path.path.segments[0].ident.to_string())
                            .and_then(|c| match c.as_str() {
                                "Ghost" => {
                                    if self.in_body == Some(CodeKind::Spec) {
                                        Some(self.mode_or_trusted(CodeKind::Spec))
                                    } else {
                                        Some(self.mode_or_trusted(CodeKind::Proof))
                                    }
                                }
                                "Tracked" => Some(self.mode_or_trusted(CodeKind::Proof)),
                                _ => None,
                            })
                        {
                            self.mark_additional_kind(&pt, wrapper_code_kind);
                        }
                    }
                }
            }
        }
    }
}

fn cut_string(s: String, len: usize) -> String {
    s.chars().take(len).collect()
}

fn hash_map_to_fit_string<V: std::fmt::Debug>(h: &[V], len: usize) -> String {
    if h.len() != 0 {
        let each_len = (len / h.len()) - 1;
        h.iter().map(|x| cut_string(format!("{:?}", x), each_len)).collect::<Vec<_>>().join(" ")
    } else {
        "".into()
    }
}

// parse the .d file and returns a vector of files names required to generate the crate
fn get_dependencies(
    dep_file_path: &std::path::Path,
) -> Result<(std::path::PathBuf, Vec<std::path::PathBuf>), String> {
    use std::path::{Path, PathBuf};

    let file = std::fs::File::open(dep_file_path)
        .map_err(|x| format!("{}, dependency file name: {:?}", x, dep_file_path))?;
    let mut reader = std::io::BufReader::new(file);
    let mut dependencies = String::new();
    std::io::BufRead::read_line(&mut reader, &mut dependencies).map_err(|x| {
        format!("Could not read the first line of the dependency file with message: {}", x)
    })?;
    let dep_file_folder = dep_file_path
        .parent()
        .ok_or(format!("invalid dependencies file path {}", dep_file_path.display()))?;
    let result: Vec<_> = dependencies
        .split_whitespace()
        .skip(1)
        .map(|x| dep_file_folder.join(Path::new(x)))
        .collect();
    assert!(result.len() > 0);

    if result.len() == 1 {
        return Ok((PathBuf::new(), vec![result[0].clone()]));
    }

    let mut path_iters: Vec<_> = result.iter().map(|x| x.iter()).collect();
    let mut chomp_components = 0;
    loop {
        let common = path_iters
            .iter_mut()
            .map(|x| x.next())
            .reduce(|acc, x| acc.and_then(|a| if Some(a) == x { Some(a) } else { None }))
            .flatten();
        if common.is_some() {
            chomp_components += 1;
        } else {
            break;
        }
    }
    let result_chomped: Vec<PathBuf> =
        result.iter().map(|p| PathBuf::from_iter(p.components().skip(chomp_components))).collect();
    let chomped = PathBuf::from_iter(result[0].iter().take(chomp_components));

    Ok((chomped, result_chomped))
}

#[derive(Debug, Clone)]
struct Summary {
    lines_by_kind: HashMap<Vec<CodeKind>, usize>,
}

impl std::ops::Add for Summary {
    type Output = Summary;

    fn add(self, rhs: Self) -> Self::Output {
        Summary {
            lines_by_kind: {
                let mut lines_by_kind = HashMap::new();
                for (kinds, count) in self.lines_by_kind.into_iter() {
                    *lines_by_kind.entry(kinds).or_default() += count;
                }
                for (kinds, count) in rhs.lines_by_kind.into_iter() {
                    *lines_by_kind.entry(kinds).or_default() += count;
                }
                lines_by_kind
            },
        }
    }
}

impl std::iter::Sum for Summary {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Summary { lines_by_kind: HashMap::new() }, |a, b| a + b)
    }
}

fn hash_set_to_sorted_vec<V: Clone + Ord>(h: &HashSet<V>) -> Vec<V> {
    let mut v: Vec<_> = h.iter().cloned().collect();
    v.sort();
    v
}

fn process_file(config: Rc<Config>, input_path: &std::path::Path) -> Result<FileStats, String> {
    let file_content = std::fs::read_to_string(input_path)
        .map_err(|e| format!("cannot read {} ({})", input_path.display(), e))?;
    let file = verus_syn::parse_file(&file_content).map_err(|e| {
        dbg!(&e.span().start(), &e.span().end());
        format!("failed to parse file {}: {}", input_path.display(), e)
    })?;

    let mut file_stats = FileStats {
        lines: file_content
            .lines()
            .map(|x| LineInfo {
                kinds: HashSet::new(),
                path: Vec::new(),
                line_content: HashSet::new(),
                text: x.into(),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    };
    let mut visitor = Visitor {
        file_stats: &mut file_stats,
        in_body: None,
        trusted: 0,
        in_proof_directive: 0,
        inside_verus_macro_or_verify_or_consider: 0,
        in_state_machine_macro: 0,
        inside_line_count_ignore_or_external: 0,
        config: config.clone(),
    };
    for attr in file.attrs.iter() {
        match &attr.meta {
            Meta::Path(path) => {
                let mut path_iter = path.segments.iter();
                match (path_iter.next(), path_iter.next()) {
                    (Some(first), Some(second))
                        if first.ident == "verus" && second.ident == "trusted" =>
                    {
                        visitor.trusted += 1;
                    }
                    _ => {}
                }
            }
            Meta::List(MetaList { path, delimiter: _, tokens: _ }) => {
                let mut path_iter = path.segments.iter();
                match (path_iter.next(), path_iter.next()) {
                    (Some(first), None) if first.ident == "cfg_attr" => {
                        let nested = attr.parse_args_with(verus_syn::punctuated::Punctuated::<Meta, verus_syn::Token![,]>::parse_terminated)
                            .map_err(|e| {
                                dbg!(&e.span().start(), &e.span().end());
                                format!("failed to parse attribute: {} {:?}", e, e.span())
                            })?;
                        let mut nested_iter = nested.iter();
                        match (nested_iter.next(), nested_iter.next()) {
                            (Some(Meta::Path(first)), Some(Meta::Path(second)))
                                if first
                                    .segments
                                    .iter()
                                    .next()
                                    .as_ref()
                                    .map(|x| x.ident == "verus_keep_ghost")
                                    .unwrap_or(false) =>
                            {
                                let mut path_iter = second.segments.iter();
                                match (path_iter.next(), path_iter.next()) {
                                    (Some(first), Some(second))
                                        if first.ident == "verus" && second.ident == "trusted" =>
                                    {
                                        visitor.trusted += 1;
                                    }
                                    _ => {}
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => (),
                }
            }
            _ => (),
        }
    }
    for item in file.items.into_iter() {
        match item {
            verus_syn::Item::Macro(ref m) => {
                if m.mac
                    .path
                    .segments
                    .last()
                    .map(|s| s.ident.to_string() == "verus")
                    .unwrap_or(false)
                {
                    let source_toks = &m.mac.tokens;
                    let macro_content: File =
                        verus_syn::parse2(source_toks.clone()).map_err(|e| {
                            dbg!(&e.span().start(), &e.span().end());
                            format!(
                                "failed to parse file {}: {} {:?}",
                                input_path.display(),
                                e,
                                e.span()
                            )
                        })?;
                    visitor.inside_verus_macro_or_verify_or_consider += 1;
                    visitor.visit_file(&macro_content);
                    visitor.inside_verus_macro_or_verify_or_consider -= 1;
                } else {
                    visitor.visit_item(&item);
                }
            }
            _ => {
                visitor.visit_item(&item);
            }
        }
    }
    let mut multiline_comment = 0;
    let mut kind_multiline_override = None;
    let override_re = regex::Regex::new(r"\$line_count\$(([A-Za-z,]*)(\$\{)?\$)|(\}\$)").unwrap();
    for line in file_stats.lines.iter_mut() {
        let trimmed = line.text.trim();
        let mut start_not_comment = (multiline_comment == 0).then(|| 0);
        let mut all_comment_indices = trimmed
            .match_indices("/*")
            .map(|(m, _)| (m, true))
            .chain(trimmed.match_indices("*/").map(|(m, _)| (m + 2, false)))
            .collect::<Vec<_>>();
        all_comment_indices.sort_by_key(|(m, _)| *m);
        let mut entirely_comment = true;
        let had_comment_start_end = all_comment_indices.len() > 0;
        for (i, s) in all_comment_indices {
            if !s {
                multiline_comment -= 1;
                if multiline_comment == 0 {
                    start_not_comment = Some(i);
                }
            } else {
                multiline_comment += 1;
                if multiline_comment == 1 {
                    if let Some(_) = start_not_comment
                        .take()
                        .map(|x| line.text[x..i].trim())
                        .filter(|x| x.is_empty())
                    {
                    } else {
                        entirely_comment = false;
                    }
                }
            }
        }
        let entirely_comment = entirely_comment && (multiline_comment > 0 || had_comment_start_end);
        if entirely_comment {
            line.line_content = HashSet::from([LineContent::Comment]);
            line.kinds = HashSet::from([CodeKind::Comment])
        }
        if trimmed.starts_with("//") {
            line.line_content = HashSet::from([LineContent::Comment]);
            line.kinds = HashSet::from([CodeKind::Comment])
        }
        if trimmed == "" {
            if !line.kinds.is_empty() {
                line.kinds = HashSet::from([CodeKind::Layout])
            }
        }
        if config.delimiters_are_layout
            && trimmed
                .chars()
                .all(|c| c == '(' || c == ')' || c == '{' || c == '}' || c == '[' || c == ']')
        {
            if !line.kinds.is_empty() {
                line.kinds = HashSet::from([CodeKind::Layout])
            }
        }
        if config.proofs_arent_trusted {
            if (line.line_content.contains(&LineContent::Body(CodeKind::Proof))
                || line.line_content.contains(&LineContent::Signature(CodeKind::Proof)))
                && line.kinds == HashSet::from([CodeKind::Trusted])
            {
                if line.line_content.contains(&LineContent::FunctionSpec) {
                    line.kinds = HashSet::from([CodeKind::Spec]);
                } else {
                    line.kinds = HashSet::from([CodeKind::Proof]);
                }
            }
        }
        if let Some(captures) = override_re.captures(trimmed) {
            if captures.get(1).is_some() {
                let kinds_str = captures.get(2).unwrap().as_str();
                let kinds = if kinds_str != "" {
                    kinds_str
                        .split(',')
                        .map(|x| match x {
                            "Trusted" => CodeKind::Trusted,
                            "Spec" => CodeKind::Spec,
                            "Proof" => CodeKind::Proof,
                            "Exec" => CodeKind::Exec,
                            "Comment" => CodeKind::Comment,
                            "Layout" => CodeKind::Layout,
                            "Directives" => CodeKind::Directives,
                            "Definitions" => CodeKind::Definitions,
                            _ => panic!("unknown code kind {}", x),
                        })
                        .collect::<HashSet<_>>()
                } else {
                    HashSet::new()
                };
                if captures.get(3).is_some() {
                    kind_multiline_override = Some(kinds);
                } else {
                    line.kinds = kinds.clone();
                }
            }
            if captures.get(4).is_some() {
                kind_multiline_override = None;
            }
        }
        if let Some(kinds) = &kind_multiline_override {
            if line.kinds != HashSet::from([CodeKind::Comment])
                && line.kinds != HashSet::from([CodeKind::Layout])
                && line.kinds != HashSet::from([])
            {
                line.kinds = kinds.clone();
            }
        }
    }
    Ok(file_stats)
}

fn warn(msg: &str) {
    eprintln!("warning: {}", msg);
}

enum RunMode<'a> {
    DepsPath(&'a std::path::Path),
    OneFile(&'a std::path::Path),
}

fn run(config: Config, run_mode_paths: RunMode<'_>) -> Result<(), String> {
    let config = Rc::new(config);
    let (root_path, files) = match run_mode_paths {
        RunMode::DepsPath(path) => get_dependencies(path)?,
        RunMode::OneFile(path) => {
            let pathd = path.display();
            (
                path.parent().ok_or_else(|| format!("invalid path {pathd}"))?.to_owned(),
                vec![std::path::PathBuf::from(
                    path.file_name().ok_or_else(|| format!("invalid path {pathd}"))?,
                )],
            )
        }
    };

    let file_stats = files
        .iter()
        .map(|f| process_file(config.clone(), &root_path.join(f)).map(|fs| (f, fs)))
        .collect::<Result<Vec<_>, String>>()?;

    if config.print_all {
        eprintln!("{:18} | {:30} | {}", "Category", "Detailed contents", "");
        eprintln!();
        for (file, file_stats) in file_stats.iter() {
            eprintln!("# {}", file.display());
            for l in file_stats.lines.iter() {
                eprintln!(
                    "{:18} | {:30} | {}",
                    hash_map_to_fit_string(&hash_set_to_sorted_vec(&l.kinds)[..], 30),
                    hash_map_to_fit_string(&hash_set_to_sorted_vec(&l.line_content)[..], 30),
                    l.text
                );
            }
            eprintln!();
        }
    }

    let file_summaries = file_stats
        .iter()
        .map(|(name, file_stats)| {
            let mut lines_by_kind = HashMap::new();
            for l in file_stats.lines.iter() {
                let mut kinds = l.kinds.clone();
                if kinds.contains(&CodeKind::Exec)
                    || kinds.contains(&CodeKind::Proof)
                    || kinds.contains(&CodeKind::Spec)
                {
                    kinds
                        .retain(|x| matches!(x, CodeKind::Exec | CodeKind::Proof | CodeKind::Spec));
                }
                *lines_by_kind.entry(hash_set_to_sorted_vec(&kinds)).or_default() += 1;
            }
            (name, Summary { lines_by_kind })
        })
        .collect::<Vec<_>>();

    let total: Summary = file_summaries.iter().map(|(_, fs)| fs).cloned().sum();

    let kinds: HashSet<_> =
        file_summaries.iter().flat_map(|(_, s)| s.lines_by_kind.keys()).cloned().collect();

    if !config.json {
        let columns: Vec<_> = {
            let mut columns: Vec<_> = vec![
                HashSet::from([CodeKind::Trusted]),
                HashSet::from([CodeKind::Spec]),
                HashSet::from([CodeKind::Proof]),
                HashSet::from([CodeKind::Exec]),
                HashSet::from([CodeKind::Proof, CodeKind::Exec]),
                HashSet::from([CodeKind::Comment]),
                HashSet::from([CodeKind::Layout]),
                HashSet::from([]),
            ];
            let other_columns: Vec<_> = kinds
                .difference(&HashSet::from_iter(columns.iter().map(hash_set_to_sorted_vec)))
                .map(|h| HashSet::from_iter(h.iter().cloned()))
                .collect();
            columns.extend(other_columns);
            columns.iter().map(hash_set_to_sorted_vec).collect()
        };

        let mut table_data: Vec<Vec<String>> = file_summaries
            .iter()
            .map(|(f, s)| {
                Some(f.display().to_string())
                    .into_iter()
                    .chain(
                        columns.iter().map(|k| format!("{}", s.lines_by_kind.get(k).unwrap_or(&0))),
                    )
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        table_data.insert(
            0,
            Some("file".to_owned())
                .into_iter()
                .chain(columns.iter().map(|k| {
                    if k.is_empty() {
                        format!("unaccounted")
                    } else {
                        k.iter().map(|x| format!("{:?}", x)).collect::<Vec<_>>().join("+")
                    }
                }))
                .collect(),
        );
        table_data.push(
            Some("total".to_owned())
                .into_iter()
                .chain(
                    columns.iter().map(|k| format!("{}", total.lines_by_kind.get(k).unwrap_or(&0))),
                )
                .collect(),
        );

        let mut table = tabled::builder::Builder::from(table_data).build();
        table
            .with(Style::markdown())
            .with(Modify::new(Columns::new(1..=kinds.len() + 1)).with(Alignment::right()))
            .with(
                Modify::new(Rows::last()).with(
                    tabled::settings::Border::default()
                        .corner_top_left('|')
                        .corner_top_right('|')
                        .top('-'),
                ),
            );
        println!("{}", table.to_string());
    } else {
        let kinds_map: HashMap<_, _> = kinds
            .iter()
            .map(|k| {
                (
                    k,
                    k.iter()
                        .map(|x| format!("{:?}", x).to_lowercase())
                        .collect::<Vec<_>>()
                        .join(","),
                )
            })
            .collect();
        let json = serde_json::json!({
            "kinds": kinds_map.iter().collect::<Vec<(_, _)>>(),
            "files": file_summaries.iter().map(|(f, s)| {
                (f.display().to_string(),
                     s.lines_by_kind.iter().map(|(k, v)| (kinds_map[k].clone(), v)).collect::<HashMap<_, _>>())
            }).collect::<HashMap<_, _>>(),
            "total": total.lines_by_kind.iter().map(|(k, v)| (kinds_map[k].clone(), v)).collect::<HashMap<_, _>>()
        });
        println!("{}", serde_json::to_string_pretty(&json).expect("invalid json"));
    }

    Ok(())
}
