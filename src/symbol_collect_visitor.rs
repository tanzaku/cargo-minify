use itertools::Itertools;
use proc_macro2::Ident;
use syn::{
    visit_mut::{
        visit_arm_mut, visit_expr_closure_mut, visit_expr_for_loop_mut, visit_expr_let_mut,
        visit_field_mut, visit_impl_item_method_mut, visit_item_const_mut, visit_item_enum_mut,
        visit_item_fn_mut, visit_item_impl_mut, visit_item_mod_mut, visit_item_static_mut,
        visit_item_struct_mut, visit_item_trait_mut, visit_local_mut, visit_trait_item_method_mut,
        visit_use_rename_mut, visit_variant_mut, VisitMut,
    },
    Arm, ExprClosure, ExprForLoop, ExprLet, Field, FnArg, ImplItemMethod, ItemConst, ItemEnum,
    ItemFn, ItemImpl, ItemMod, ItemStatic, ItemStruct, ItemTrait, Local, Pat, TraitItemMethod,
    UseRename, Variant,
};

pub struct SymbolCollectVisitor {
    pub ident_var: Vec<Ident>,
    pub ident_others: Vec<Ident>,
    impl_for_trait_stack: Vec<bool>,
}

impl SymbolCollectVisitor {
    pub fn new() -> Self {
        Self {
            ident_var: Vec::new(),
            ident_others: Vec::new(),
            impl_for_trait_stack: Vec::new(),
        }
    }

    fn pat_to_ident(pat: &Pat) -> Vec<Ident> {
        match pat {
            Pat::Ident(pat_ident) => vec![pat_ident.ident.clone()],
            Pat::TupleStruct(pat_tuple_struct) => pat_tuple_struct
                .pat
                .elems
                .iter()
                .flat_map(|pat| Self::pat_to_ident(pat))
                .collect_vec(),
            Pat::Reference(pat_reference) => Self::pat_to_ident(&pat_reference.pat),
            Pat::Type(pat_type) => Self::pat_to_ident(&pat_type.pat),
            Pat::Tuple(pat_tuple) => pat_tuple
                .elems
                .iter()
                .flat_map(Self::pat_to_ident)
                .collect_vec(),
            _ => Vec::new(),
        }
    }
}

impl VisitMut for SymbolCollectVisitor {
    fn visit_item_mod_mut(&mut self, node: &mut ItemMod) {
        self.ident_others.push(node.ident.clone());
        visit_item_mod_mut(self, node);
    }

    fn visit_use_rename_mut(&mut self, node: &mut UseRename) {
        self.ident_others.push(node.rename.clone());
        visit_use_rename_mut(self, node);
    }

    fn visit_item_struct_mut(&mut self, node: &mut ItemStruct) {
        self.ident_others.push(node.ident.clone());
        visit_item_struct_mut(self, node);
    }

    fn visit_field_mut(&mut self, node: &mut Field) {
        if let Some(ident) = node.ident.as_ref() {
            self.ident_others.push(ident.clone());
        }
        visit_field_mut(self, node);
    }

    fn visit_item_enum_mut(&mut self, node: &mut ItemEnum) {
        self.ident_others.push(node.ident.clone());
        visit_item_enum_mut(self, node);
    }

    fn visit_variant_mut(&mut self, node: &mut Variant) {
        self.ident_others.push(node.ident.clone());
        visit_variant_mut(self, node);
    }

    fn visit_arm_mut(&mut self, node: &mut Arm) {
        self.ident_var.extend(Self::pat_to_ident(&node.pat));
        visit_arm_mut(self, node);
    }

    fn visit_item_fn_mut(&mut self, node: &mut ItemFn) {
        self.ident_others.push(node.sig.ident.clone());

        node.sig.inputs.iter().for_each(|fn_arg| match fn_arg {
            FnArg::Typed(pat_type) => {
                self.ident_var.extend(Self::pat_to_ident(&pat_type.pat));
            }
            _ => (),
        });

        visit_item_fn_mut(self, node);
    }

    fn visit_expr_closure_mut(&mut self, node: &mut ExprClosure) {
        node.inputs
            .iter()
            .for_each(|pat| self.ident_var.extend(Self::pat_to_ident(pat)));
        visit_expr_closure_mut(self, node);
    }

    fn visit_item_impl_mut(&mut self, node: &mut ItemImpl) {
        self.impl_for_trait_stack.push(node.trait_.is_some());
        visit_item_impl_mut(self, node);
        self.impl_for_trait_stack.pop();
    }

    fn visit_impl_item_method_mut(&mut self, node: &mut ImplItemMethod) {
        // traitを実装している場合、メソッド名は変更できない
        if !*self.impl_for_trait_stack.last().unwrap() {
            self.ident_others.push(node.sig.ident.clone());
        }

        node.sig.inputs.iter().for_each(|fn_arg| match fn_arg {
            FnArg::Typed(pat_type) => {
                self.ident_var.extend(Self::pat_to_ident(&pat_type.pat));
            }
            _ => (),
        });

        visit_impl_item_method_mut(self, node);
    }

    fn visit_item_trait_mut(&mut self, node: &mut ItemTrait) {
        self.ident_others.push(node.ident.clone());
        visit_item_trait_mut(self, node);
    }

    fn visit_trait_item_method_mut(&mut self, node: &mut TraitItemMethod) {
        self.ident_others.push(node.sig.ident.clone());

        node.sig.inputs.iter().for_each(|fn_arg| match fn_arg {
            FnArg::Typed(pat_type) => {
                self.ident_var.extend(Self::pat_to_ident(&pat_type.pat));
            }
            _ => (),
        });

        visit_trait_item_method_mut(self, node);
    }

    // const
    fn visit_item_const_mut(&mut self, node: &mut ItemConst) {
        self.ident_var.push(node.ident.clone());
        visit_item_const_mut(self, node);
    }

    // static変数
    fn visit_item_static_mut(&mut self, node: &mut ItemStatic) {
        self.ident_var.push(node.ident.clone());
        visit_item_static_mut(self, node);
    }

    // ローカル変数
    fn visit_local_mut(&mut self, node: &mut Local) {
        self.ident_var.extend(Self::pat_to_ident(&node.pat));
        visit_local_mut(self, node);
    }

    fn visit_expr_for_loop_mut(&mut self, node: &mut ExprForLoop) {
        self.ident_var.extend(Self::pat_to_ident(&node.pat));
        visit_expr_for_loop_mut(self, node);
    }

    fn visit_expr_let_mut(&mut self, node: &mut ExprLet) {
        self.ident_var.extend(Self::pat_to_ident(&node.pat));
        visit_expr_let_mut(self, node);
    }
}
