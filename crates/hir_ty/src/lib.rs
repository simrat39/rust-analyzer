//! The type system. We currently use this to infer types for completion, hover
//! information and various assists.

#[allow(unused)]
macro_rules! eprintln {
    ($($tt:tt)*) => { stdx::eprintln!($($tt)*) };
}

mod autoderef;
mod builder;
mod chalk_db;
mod chalk_ext;
pub mod consteval;
mod infer;
mod interner;
mod lower;
mod mapping;
mod op;
mod tls;
mod utils;
mod walk;
pub mod db;
pub mod diagnostics;
pub mod display;
pub mod method_resolution;
pub mod primitive;
pub mod traits;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod test_db;

use std::sync::Arc;

use chalk_ir::{
    fold::{Fold, Shift},
    interner::HasInterner,
    UintTy,
};
use hir_def::{
    expr::ExprId,
    type_ref::{ConstScalar, Rawness},
    TypeParamId,
};
use stdx::always;

use crate::{db::HirDatabase, utils::generics};

pub use autoderef::autoderef;
pub use builder::TyBuilder;
pub use chalk_ext::*;
pub use infer::{could_unify, InferenceResult};
pub use interner::Interner;
pub use lower::{
    associated_type_shorthand_candidates, callable_item_sig, CallableDefId, ImplTraitLoweringMode,
    TyDefId, TyLoweringContext, ValueTyDefId,
};
pub use mapping::{
    const_from_placeholder_idx, from_assoc_type_id, from_chalk_trait_id, from_foreign_def_id,
    from_placeholder_idx, lt_from_placeholder_idx, to_assoc_type_id, to_chalk_trait_id,
    to_foreign_def_id, to_placeholder_idx,
};
pub use traits::TraitEnvironment;
pub use utils::all_super_traits;
pub use walk::TypeWalk;

pub use chalk_ir::{
    cast::Cast, AdtId, BoundVar, DebruijnIndex, Mutability, Safety, Scalar, TyVariableKind,
};

pub type ForeignDefId = chalk_ir::ForeignDefId<Interner>;
pub type AssocTypeId = chalk_ir::AssocTypeId<Interner>;
pub type FnDefId = chalk_ir::FnDefId<Interner>;
pub type ClosureId = chalk_ir::ClosureId<Interner>;
pub type OpaqueTyId = chalk_ir::OpaqueTyId<Interner>;
pub type PlaceholderIndex = chalk_ir::PlaceholderIndex;

pub type VariableKind = chalk_ir::VariableKind<Interner>;
pub type VariableKinds = chalk_ir::VariableKinds<Interner>;
pub type CanonicalVarKinds = chalk_ir::CanonicalVarKinds<Interner>;
pub type Binders<T> = chalk_ir::Binders<T>;
pub type Substitution = chalk_ir::Substitution<Interner>;
pub type GenericArg = chalk_ir::GenericArg<Interner>;
pub type GenericArgData = chalk_ir::GenericArgData<Interner>;

pub type Ty = chalk_ir::Ty<Interner>;
pub type TyKind = chalk_ir::TyKind<Interner>;
pub type DynTy = chalk_ir::DynTy<Interner>;
pub type FnPointer = chalk_ir::FnPointer<Interner>;
// pub type FnSubst = chalk_ir::FnSubst<Interner>;
pub use chalk_ir::FnSubst;
pub type ProjectionTy = chalk_ir::ProjectionTy<Interner>;
pub type AliasTy = chalk_ir::AliasTy<Interner>;
pub type OpaqueTy = chalk_ir::OpaqueTy<Interner>;
pub type InferenceVar = chalk_ir::InferenceVar;

pub type Lifetime = chalk_ir::Lifetime<Interner>;
pub type LifetimeData = chalk_ir::LifetimeData<Interner>;
pub type LifetimeOutlives = chalk_ir::LifetimeOutlives<Interner>;

pub type Const = chalk_ir::Const<Interner>;
pub type ConstData = chalk_ir::ConstData<Interner>;
pub type ConstValue = chalk_ir::ConstValue<Interner>;
pub type ConcreteConst = chalk_ir::ConcreteConst<Interner>;

pub type ChalkTraitId = chalk_ir::TraitId<Interner>;
pub type TraitRef = chalk_ir::TraitRef<Interner>;
pub type QuantifiedWhereClause = Binders<WhereClause>;
pub type QuantifiedWhereClauses = chalk_ir::QuantifiedWhereClauses<Interner>;
pub type Canonical<T> = chalk_ir::Canonical<T>;

pub type FnSig = chalk_ir::FnSig<Interner>;

pub type InEnvironment<T> = chalk_ir::InEnvironment<T>;
pub type DomainGoal = chalk_ir::DomainGoal<Interner>;
pub type Goal = chalk_ir::Goal<Interner>;
pub type AliasEq = chalk_ir::AliasEq<Interner>;
pub type Solution = chalk_solve::Solution<Interner>;
pub type ConstrainedSubst = chalk_ir::ConstrainedSubst<Interner>;
pub type Guidance = chalk_solve::Guidance<Interner>;
pub type WhereClause = chalk_ir::WhereClause<Interner>;

// FIXME: get rid of this
pub fn subst_prefix(s: &Substitution, n: usize) -> Substitution {
    Substitution::from_iter(
        &Interner,
        s.as_slice(&Interner)[..std::cmp::min(s.len(&Interner), n)].iter().cloned(),
    )
}

/// Return an index of a parameter in the generic type parameter list by it's id.
pub fn param_idx(db: &dyn HirDatabase, id: TypeParamId) -> Option<usize> {
    generics(db.upcast(), id.parent).param_idx(id)
}

pub(crate) fn wrap_empty_binders<T>(value: T) -> Binders<T>
where
    T: Fold<Interner, Result = T> + HasInterner<Interner = Interner>,
{
    Binders::empty(&Interner, value.shifted_in_from(&Interner, DebruijnIndex::ONE))
}

pub(crate) fn make_only_type_binders<T: HasInterner<Interner = Interner>>(
    num_vars: usize,
    value: T,
) -> Binders<T> {
    Binders::new(
        VariableKinds::from_iter(
            &Interner,
            std::iter::repeat(chalk_ir::VariableKind::Ty(chalk_ir::TyVariableKind::General))
                .take(num_vars),
        ),
        value,
    )
}

// FIXME: get rid of this
pub fn make_canonical<T: HasInterner<Interner = Interner>>(
    value: T,
    kinds: impl IntoIterator<Item = TyVariableKind>,
) -> Canonical<T> {
    let kinds = kinds.into_iter().map(|tk| {
        chalk_ir::CanonicalVarKind::new(
            chalk_ir::VariableKind::Ty(tk),
            chalk_ir::UniverseIndex::ROOT,
        )
    });
    Canonical { value, binders: chalk_ir::CanonicalVarKinds::from_iter(&Interner, kinds) }
}

// FIXME: get rid of this, just replace it by FnPointer
/// A function signature as seen by type inference: Several parameter types and
/// one return type.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallableSig {
    params_and_return: Arc<[Ty]>,
    is_varargs: bool,
}

has_interner!(CallableSig);

/// A polymorphic function signature.
pub type PolyFnSig = Binders<CallableSig>;

impl CallableSig {
    pub fn from_params_and_return(mut params: Vec<Ty>, ret: Ty, is_varargs: bool) -> CallableSig {
        params.push(ret);
        CallableSig { params_and_return: params.into(), is_varargs }
    }

    pub fn from_fn_ptr(fn_ptr: &FnPointer) -> CallableSig {
        CallableSig {
            // FIXME: what to do about lifetime params? -> return PolyFnSig
            params_and_return: fn_ptr
                .substitution
                .clone()
                .shifted_out_to(&Interner, DebruijnIndex::ONE)
                .expect("unexpected lifetime vars in fn ptr")
                .0
                .as_slice(&Interner)
                .iter()
                .map(|arg| arg.assert_ty_ref(&Interner).clone())
                .collect(),
            is_varargs: fn_ptr.sig.variadic,
        }
    }

    pub fn to_fn_ptr(&self) -> FnPointer {
        FnPointer {
            num_binders: 0,
            sig: FnSig { abi: (), safety: Safety::Safe, variadic: self.is_varargs },
            substitution: FnSubst(Substitution::from_iter(
                &Interner,
                self.params_and_return.iter().cloned(),
            )),
        }
    }

    pub fn params(&self) -> &[Ty] {
        &self.params_and_return[0..self.params_and_return.len() - 1]
    }

    pub fn ret(&self) -> &Ty {
        &self.params_and_return[self.params_and_return.len() - 1]
    }
}

impl Fold<Interner> for CallableSig {
    type Result = CallableSig;

    fn fold_with<'i>(
        self,
        folder: &mut dyn chalk_ir::fold::Folder<'i, Interner>,
        outer_binder: DebruijnIndex,
    ) -> chalk_ir::Fallible<Self::Result>
    where
        Interner: 'i,
    {
        let vec = self.params_and_return.to_vec();
        let folded = vec.fold_with(folder, outer_binder)?;
        Ok(CallableSig { params_and_return: folded.into(), is_varargs: self.is_varargs })
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum ImplTraitId {
    ReturnTypeImplTrait(hir_def::FunctionId, u16),
    AsyncBlockTypeImplTrait(hir_def::DefWithBodyId, ExprId),
}

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct ReturnTypeImplTraits {
    pub(crate) impl_traits: Vec<ReturnTypeImplTrait>,
}

has_interner!(ReturnTypeImplTraits);

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub(crate) struct ReturnTypeImplTrait {
    pub(crate) bounds: Binders<Vec<QuantifiedWhereClause>>,
}

pub fn static_lifetime() -> Lifetime {
    LifetimeData::Static.intern(&Interner)
}

pub fn dummy_usize_const() -> Const {
    let usize_ty = chalk_ir::TyKind::Scalar(Scalar::Uint(UintTy::Usize)).intern(&Interner);
    chalk_ir::ConstData {
        ty: usize_ty,
        value: chalk_ir::ConstValue::Concrete(chalk_ir::ConcreteConst {
            interned: ConstScalar::Unknown,
        }),
    }
    .intern(&Interner)
}

pub(crate) fn fold_free_vars<T: HasInterner<Interner = Interner> + Fold<Interner>>(
    t: T,
    f: impl FnMut(BoundVar, DebruijnIndex) -> Ty,
) -> T::Result {
    use chalk_ir::{fold::Folder, Fallible};
    struct FreeVarFolder<F>(F);
    impl<'i, F: FnMut(BoundVar, DebruijnIndex) -> Ty + 'i> Folder<'i, Interner> for FreeVarFolder<F> {
        fn as_dyn(&mut self) -> &mut dyn Folder<'i, Interner> {
            self
        }

        fn interner(&self) -> &'i Interner {
            &Interner
        }

        fn fold_free_var_ty(
            &mut self,
            bound_var: BoundVar,
            outer_binder: DebruijnIndex,
        ) -> Fallible<Ty> {
            Ok(self.0(bound_var, outer_binder))
        }
    }
    t.fold_with(&mut FreeVarFolder(f), DebruijnIndex::INNERMOST).expect("fold failed unexpectedly")
}

pub(crate) fn fold_tys<T: HasInterner<Interner = Interner> + Fold<Interner>>(
    t: T,
    f: impl FnMut(Ty, DebruijnIndex) -> Ty,
    binders: DebruijnIndex,
) -> T::Result {
    use chalk_ir::{
        fold::{Folder, SuperFold},
        Fallible,
    };
    struct TyFolder<F>(F);
    impl<'i, F: FnMut(Ty, DebruijnIndex) -> Ty + 'i> Folder<'i, Interner> for TyFolder<F> {
        fn as_dyn(&mut self) -> &mut dyn Folder<'i, Interner> {
            self
        }

        fn interner(&self) -> &'i Interner {
            &Interner
        }

        fn fold_ty(&mut self, ty: Ty, outer_binder: DebruijnIndex) -> Fallible<Ty> {
            let ty = ty.super_fold_with(self.as_dyn(), outer_binder)?;
            Ok(self.0(ty, outer_binder))
        }
    }
    t.fold_with(&mut TyFolder(f), binders).expect("fold failed unexpectedly")
}

pub fn replace_errors_with_variables<T>(t: T) -> Canonical<T::Result>
where
    T: HasInterner<Interner = Interner> + Fold<Interner>,
    T::Result: HasInterner<Interner = Interner>,
{
    use chalk_ir::{
        fold::{Folder, SuperFold},
        Fallible,
    };
    struct ErrorReplacer {
        vars: usize,
    }
    impl<'i> Folder<'i, Interner> for ErrorReplacer {
        fn as_dyn(&mut self) -> &mut dyn Folder<'i, Interner> {
            self
        }

        fn interner(&self) -> &'i Interner {
            &Interner
        }

        fn fold_ty(&mut self, ty: Ty, outer_binder: DebruijnIndex) -> Fallible<Ty> {
            if let TyKind::Error = ty.kind(&Interner) {
                let index = self.vars;
                self.vars += 1;
                Ok(TyKind::BoundVar(BoundVar::new(outer_binder, index)).intern(&Interner))
            } else {
                let ty = ty.super_fold_with(self.as_dyn(), outer_binder)?;
                Ok(ty)
            }
        }

        fn fold_inference_ty(
            &mut self,
            var: InferenceVar,
            kind: TyVariableKind,
            _outer_binder: DebruijnIndex,
        ) -> Fallible<Ty> {
            always!(false);
            Ok(TyKind::InferenceVar(var, kind).intern(&Interner))
        }
    }
    let mut error_replacer = ErrorReplacer { vars: 0 };
    let value = t
        .fold_with(&mut error_replacer, DebruijnIndex::INNERMOST)
        .expect("fold failed unexpectedly");
    let kinds = (0..error_replacer.vars).map(|_| {
        chalk_ir::CanonicalVarKind::new(
            chalk_ir::VariableKind::Ty(TyVariableKind::General),
            chalk_ir::UniverseIndex::ROOT,
        )
    });
    Canonical { value, binders: chalk_ir::CanonicalVarKinds::from_iter(&Interner, kinds) }
}
