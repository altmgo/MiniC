//! Type checker implementation for MiniC.
//!
//! # Overview
//!
//! Provides [`type_check`], which walks an [`UncheckedProgram`] and either
//! returns a [`CheckedProgram`] (every node annotated with its [`Type`]) or
//! a [`TypeError`] describing the first violation found.
//!
//! Also defines [`TypeError`], the error type returned on failure.
//!
//! # Design Decisions
//!
//! ## Using `Environment<Type>` for variable tracking
//!
//! The type checker stores the *declared type* of every in-scope name in an
//! [`Environment<Type>`](crate::environment::Environment). Here `Type` is the
//! MiniC type (e.g., `Type::Int`), not a Rust type. This is the same
//! `Environment` struct used by the interpreter — but instantiated with
//! `Type` instead of `Value`. Functions are also stored in this environment
//! as `Type::Fun(param_types, return_type)`, so the same lookup mechanism
//! handles both variable and function name resolution.
//!
//! ## Function signatures registered before bodies are checked
//!
//! All function signatures are added to the environment before any function
//! body is checked. This allows functions to call each other (mutual
//! recursion) without requiring forward declarations. A `fn_snapshot` of the
//! function-only environment is taken after this step and restored at the
//! start of each function body check, ensuring variable bindings from one
//! function do not leak into another.
//!
//! ## Block scoping via `snapshot` / `restore`
//!
//! When the type checker enters a block statement, it takes a snapshot of the
//! current environment. When the block exits (normally or via early return),
//! it restores the snapshot, discarding any variables declared inside. This
//! correctly implements lexical block scoping without a separate scope-stack
//! data structure.
//!
//! ## `Type::Any` and `types_compatible`
//!
//! The `types_compatible` function implements MiniC's assignability rules,
//! including `Int`↔`Float` coercion and the `Any` wildcard used by `print`.
//! Centralising compatibility logic here means all callers (declaration,
//! assignment, call-argument checking) share one consistent definition.

use std::collections::{HashMap, HashSet};

use crate::environment::{build_type_decl_map, Environment};
use crate::ir::ast::{
    CheckedExpr, CheckedFunDecl, CheckedProgram, CheckedStmt, Expr, ExprD, FunDecl, Literal,
    MatchArm, Program, Statement, StatementD, Type, UncheckedExpr, UncheckedFunDecl,
    UncheckedProgram, UncheckedStmt, UserTypeDecl, UserTypeKind, UserTypeMember,
};
use crate::stdlib::NativeRegistry;

/// A type error reported by the type checker.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeError {
    pub message: String,
}

impl TypeError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TypeError {}

fn check_type_decls_unique(decls: &[UserTypeDecl]) -> Result<(), TypeError> {
    let mut seen = HashSet::new();
    for decl in decls {
        let key = (decl.specifier.clone(), decl.identifier.clone());
        if !seen.insert(key) {
            return Err(TypeError::new(format!(
                "duplicate type declaration: {:?} {}",
                decl.specifier, decl.identifier
            )));
        }
    }
    Ok(())
}

/// Type-check a program. Returns `Ok(CheckedProgram)` if well-typed, `Err(TypeError)` on first error.
/// Requires a `main` function with signature `void main()`.
pub fn type_check(program: &UncheckedProgram) -> Result<CheckedProgram, TypeError> {
    check_type_decls_unique(&program.type_declarations)?;
    let type_map = build_type_decl_map(&program.type_declarations);

    let main_fn = program.main_function();
    match main_fn {
        None => return Err(TypeError::new("program must have a main function")),
        Some(f) => {
            if f.return_type != Type::Unit {
                return Err(TypeError::new("main function must return void"));
            }
            if !f.params.is_empty() {
                return Err(TypeError::new("main function must have no parameters"));
            }
        }
    }

    let mut env = Environment::with_type_decls(type_map);

    // Register native stdlib functions as Type::Function bindings.
    let registry = NativeRegistry::default();
    for (name, entry) in registry.iter() {
        env.declare(
            name.clone(),
            Type::Function {
                params: entry.params.clone(),
                return_type: Box::new(entry.return_type.clone()),
            },
        );
    }

    // Register user-defined function signatures as Type::Function bindings.
    for f in &program.functions {
        let param_tys = f.params.iter().map(|param| param.ty.clone()).collect();
        env.declare(
            f.name.clone(),
            Type::Function {
                params: param_tys,
                return_type: Box::new(f.return_type.clone()),
            },
        );
    }

    // Clean snapshot: only function bindings, no variable bindings.
    let fn_snapshot = env.snapshot();

    let mut functions = Vec::new();
    for f in &program.functions {
        let checked = type_check_fun_decl(f, &mut env, &fn_snapshot)?;
        functions.push(checked);
    }
    Ok(Program {
        type_declarations: program.type_declarations.clone(),
        functions,
    })
}

fn type_check_fun_decl(
    f: &UncheckedFunDecl,
    env: &mut Environment<Type>,
    fn_snapshot: &HashMap<String, Type>,
) -> Result<CheckedFunDecl, TypeError> {
    // Restore to clean function-only state, then add parameters.
    env.restore(fn_snapshot.clone());
    for param in &f.params {
        env.declare(param.name.clone(), param.ty.clone());
    }
    let body = type_check_stmt(&f.body, env, &f.return_type)?;
    Ok(FunDecl {
        name: f.name.clone(),
        params: f.params.clone(),
        return_type: f.return_type.clone(),
        body: Box::new(body),
    })
}

fn type_check_stmt(
    s: &UncheckedStmt,
    env: &mut Environment<Type>,
    expected_return: &Type,
) -> Result<CheckedStmt, TypeError> {
    let stmt = match &s.stmt {
        Statement::Decl { name, ty, init } => {
            if ty == &Type::Unit {
                return Err(TypeError::new("cannot declare variable of type void"));
            }
            match ty {
                Type::Struct(identifier) => {
                    if !env.has_type_decl(&UserTypeKind::Struct, identifier) {
                        return Err(TypeError::new(format!(
                            "unknown struct type: {}",
                            identifier
                        )));
                    }
                }
                Type::Enum(identifier) => {
                    if !env.has_type_decl(&UserTypeKind::Enum, identifier) {
                        return Err(TypeError::new(format!(
                            "unknown enum type: {}",
                            identifier
                        )));
                    }
                }
                _ => {}
            }
            if env.get(name).is_some() {
                return Err(TypeError::new(format!(
                    "redeclaration of variable: {}",
                    name
                )));
            }
            let init_checked = match ty {
                Type::Struct(struct_name) => type_check_struct_init(init, struct_name, env)?,
                Type::Enum(enum_name) => type_check_enum_init(init, enum_name, env)?,
                _ => type_check_expr_to_typed(init, env)?,
            };
            if !types_compatible(&init_checked.ty, ty) {
                return Err(TypeError::new(format!(
                    "declaration of {}: expected {:?}, got {:?}",
                    name, ty, init_checked.ty
                )));
            }
            env.declare(name.clone(), ty.clone());
            Statement::Decl {
                name: name.clone(),
                ty: ty.clone(),
                init: Box::new(init_checked),
            }
        }
        Statement::Assign { target, value } => {
            let value_checked = type_check_expr_to_typed(value, env)?;
            type_check_assign_target(&target.exp, &value_checked.ty, env)?;
            Statement::Assign {
                target: Box::new(type_check_expr_to_typed(target, env)?),
                value: Box::new(value_checked),
            }
        }
        Statement::Block { seq } => {
            let snapshot = env.snapshot();
            let mut checked = Vec::new();
            for st in seq {
                checked.push(type_check_stmt(st, env, expected_return)?);
            }
            env.restore(snapshot);
            Statement::Block { seq: checked }
        }
        Statement::Call { name, args } => {
            let args_checked: Result<Vec<_>, _> = args
                .iter()
                .map(|a| type_check_expr_to_typed(a, env))
                .collect();
            let args_checked = args_checked?;
            check_call(name, &args_checked, env)?;
            Statement::Call {
                name: name.clone(),
                args: args_checked,
            }
        }
        Statement::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let cond_checked = type_check_expr_to_typed(cond, env)?;
            if cond_checked.ty != Type::Bool {
                return Err(TypeError::new(format!(
                    "if condition must be Bool, got {:?}",
                    cond_checked.ty
                )));
            }
            let then_checked = type_check_stmt(then_branch, env, expected_return)?;
            let else_checked = else_branch
                .as_ref()
                .map(|e| type_check_stmt(e, env, expected_return))
                .transpose()?;
            Statement::If {
                cond: Box::new(cond_checked),
                then_branch: Box::new(then_checked),
                else_branch: else_checked.map(Box::new),
            }
        }
        Statement::While { cond, body } => {
            let cond_checked = type_check_expr_to_typed(cond, env)?;
            if cond_checked.ty != Type::Bool {
                return Err(TypeError::new(format!(
                    "while condition must be Bool, got {:?}",
                    cond_checked.ty
                )));
            }
            let body_checked = type_check_stmt(body, env, expected_return)?;
            Statement::While {
                cond: Box::new(cond_checked),
                body: Box::new(body_checked),
            }
        }
        Statement::Return(expr) => match expr {
            None => {
                if *expected_return != Type::Unit {
                    return Err(TypeError::new(format!(
                        "non-void function must return a value of type {:?}",
                        expected_return
                    )));
                }
                Statement::Return(None)
            }
            Some(e) => {
                if *expected_return == Type::Unit {
                    return Err(TypeError::new("void function must not return a value"));
                }
                let checked = type_check_expr_to_typed(e, env)?;
                if !types_compatible(&checked.ty, expected_return) {
                    return Err(TypeError::new(format!(
                        "return type mismatch: expected {:?}, got {:?}",
                        expected_return, checked.ty
                    )));
                }
                Statement::Return(Some(Box::new(checked)))
            }
        }
        Statement::Match { target, arms } => {
            let target_checked = type_check_expr_to_typed(target, env)?;
            let enum_name = match &target_checked.ty {
                Type::Enum(name) => name.clone(),
                other => {
                    return Err(TypeError::new(format!(
                        "match target must be an enum, got {:?}",
                        other
                    )))
                }
            };
            let decl = env
                .get_type_decl(&UserTypeKind::Enum, &enum_name)
                .ok_or_else(|| {
                    TypeError::new(format!("unknown enum type in match: {}", enum_name))
                })?
                .clone();
            let mut checked_arms = Vec::new();
            for arm in arms {
                let payload_ty = decl.members.iter().find_map(|m| match m {
                    UserTypeMember::EnumVariant { name, ty }
                        if *name == arm.variant =>
                    {
                        Some(ty.clone())
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    TypeError::new(format!(
                        "unknown variant '{}' for enum {}",
                        arm.variant, enum_name
                    ))
                })?;
                let snapshot = env.snapshot();
                if let Some(ref pty) = payload_ty {
                    env.declare(arm.variant.clone(), pty.clone());
                }
                let body_checked = type_check_stmt(&arm.body, env, expected_return)?;
                env.restore(snapshot);
                checked_arms.push(MatchArm {
                    variant: arm.variant.clone(),
                    binding: payload_ty.as_ref().map(|_| arm.variant.clone()),
                    body: Box::new(body_checked),
                });
            }
            Statement::Match {
                target: Box::new(target_checked),
                arms: checked_arms,
            }
        }
    };
    Ok(StatementD {
        stmt,
        ty: Type::Unit,
    })
}

fn check_call(name: &str, args: &[CheckedExpr], env: &Environment<Type>) -> Result<(), TypeError> {
    match env.get(name) {
        Some(Type::Function {
            params: param_tys, ..
        }) => {
            if args.len() != param_tys.len() {
                return Err(TypeError::new(format!(
                    "function '{}' expects {} arguments, got {}",
                    name,
                    param_tys.len(),
                    args.len()
                )));
            }
            for (i, (arg, param_ty)) in args.iter().zip(param_tys.iter()).enumerate() {
                if !types_compatible(&arg.ty, param_ty) {
                    return Err(TypeError::new(format!(
                        "argument {} to {}: expected {:?}, got {:?}",
                        i + 1,
                        name,
                        param_ty,
                        arg.ty
                    )));
                }
            }
            Ok(())
        }
        Some(_) => Err(TypeError::new(format!("'{}' is not a function", name))),
        None => Err(TypeError::new(format!("undefined function: {}", name))),
    }
}

fn type_check_assign_target(
    target: &Expr<()>,
    value_ty: &Type,
    env: &Environment<Type>,
) -> Result<(), TypeError> {
    match target {
        Expr::Ident(name) => {
            let declared_ty = env
                .get(name)
                .ok_or_else(|| TypeError::new(format!("undeclared variable: {}", name)))?;
            if !types_compatible(value_ty, declared_ty) {
                return Err(TypeError::new(format!(
                    "assignment to {}: expected {:?}, got {:?}",
                    name, declared_ty, value_ty
                )));
            }
            Ok(())
        }
        Expr::Index { base, index } => {
            let index_ty = type_check_expr(index, env)?;
            if index_ty != Type::Int {
                return Err(TypeError::new("array index must be Int"));
            }
            let base_ty = type_check_expr(base, env)?;
            if let Type::Array(elem) = &base_ty {
                if **elem != *value_ty {
                    return Err(TypeError::new("assignment type mismatch"));
                }
            } else {
                return Err(TypeError::new("indexed target must be array"));
            }
            Ok(())
        }
        Expr::Member { base, member } => {
            let base_ty = type_check_expr(base, env)?;
            match base_ty {
                Type::Struct(ref identifier) => {
                    let decl = env
                        .get_type_decl(&UserTypeKind::Struct, identifier)
                        .ok_or_else(|| {
                            TypeError::new(format!(
                                "unknown struct type in member assignment: {}",
                                identifier
                            ))
                        })?;

                    let field_ty = decl
                        .members
                        .iter()
                        .find_map(|m| match m {
                            UserTypeMember::Field(decl) if decl.name == *member => {
                                Some(decl.ty.clone())
                            }
                            _ => None,
                        })
                        .ok_or_else(|| {
                            TypeError::new(format!(
                                "unknown member '{}' on struct {}",
                                member, identifier
                            ))
                        })?;

                    if !types_compatible(value_ty, &field_ty) {
                        return Err(TypeError::new(format!(
                            "assignment to {}.{}: expected {:?}, got {:?}",
                            identifier, member, field_ty, value_ty
                        )));
                    }
                    Ok(())
                }
                Type::Enum(_) => Err(TypeError::new("cannot assign to enum members")),
                other => Err(TypeError::new(format!(
                    "member assignment requires struct base type, got {:?}",
                    other
                ))),
            }
        }
        _ => Err(TypeError::new("invalid assignment target")),
    }
}

fn type_check_expr_to_typed(
    e: &UncheckedExpr,
    env: &Environment<Type>,
) -> Result<CheckedExpr, TypeError> {
    let ty = type_check_expr(e, env)?;
    let exp = type_check_expr_inner(&e.exp, env)?;
    Ok(ExprD { exp, ty })
}

fn type_check_expr_inner(e: &Expr<()>, env: &Environment<Type>) -> Result<Expr<Type>, TypeError> {
    match e {
        Expr::Literal(l) => Ok(Expr::Literal(l.clone())),
        Expr::Ident(name) => Ok(Expr::Ident(name.clone())),
        Expr::Neg(inner) => Ok(Expr::Neg(Box::new(type_check_expr_to_typed(inner, env)?))),
        Expr::Add(l, r) => Ok(Expr::Add(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Sub(l, r) => Ok(Expr::Sub(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Mul(l, r) => Ok(Expr::Mul(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Div(l, r) => Ok(Expr::Div(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Eq(l, r) => Ok(Expr::Eq(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Ne(l, r) => Ok(Expr::Ne(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Lt(l, r) => Ok(Expr::Lt(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Le(l, r) => Ok(Expr::Le(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Gt(l, r) => Ok(Expr::Gt(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Ge(l, r) => Ok(Expr::Ge(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Not(inner) => Ok(Expr::Not(Box::new(type_check_expr_to_typed(inner, env)?))),
        Expr::And(l, r) => Ok(Expr::And(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Or(l, r) => Ok(Expr::Or(
            Box::new(type_check_expr_to_typed(l, env)?),
            Box::new(type_check_expr_to_typed(r, env)?),
        )),
        Expr::Call { name, args } => {
            let args_checked: Result<Vec<_>, _> = args
                .iter()
                .map(|a| type_check_expr_to_typed(a, env))
                .collect();
            Ok(Expr::Call {
                name: name.clone(),
                args: args_checked?,
            })
        }
        Expr::ArrayLit(elems) => {
            let elems_checked: Result<Vec<_>, _> = elems
                .iter()
                .map(|e| type_check_expr_to_typed(e, env))
                .collect();
            Ok(Expr::ArrayLit(elems_checked?))
        }
        Expr::Index { base, index } => Ok(Expr::Index {
            base: Box::new(type_check_expr_to_typed(base, env)?),
            index: Box::new(type_check_expr_to_typed(index, env)?),
        }),
        Expr::Member { base, member } => Ok(Expr::Member {
            base: Box::new(type_check_expr_to_typed(base, env)?),
            member: member.clone(),
        }),
        Expr::Init { .. } => Err(TypeError::new(
            "struct/enum init used outside of variable declaration",
        )),
        Expr::Cast { ty, expr } => {
            let checked_inner = match ty {
                Type::Enum(enum_name) => {
                    resolve_enum_variant_expr(enum_name, expr, env)?
                }
                _ => type_check_expr_to_typed(expr, env)?,
            };
            Ok(Expr::Cast {
                ty: ty.clone(),
                expr: Box::new(checked_inner),
            })
        }
        Expr::EnumVariant { enum_name, variant, payload } => {
            let checked_payload = match payload {
                Some(e) => Some(Box::new(type_check_expr_to_typed(e, env)?)),
                None => None,
            };
            Ok(Expr::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                payload: checked_payload,
            })
        }
    }
}

fn resolve_enum_variant_expr(
    enum_name: &str,
    expr: &UncheckedExpr,
    env: &Environment<Type>,
) -> Result<CheckedExpr, TypeError> {
    let decl = env
        .get_type_decl(&UserTypeKind::Enum, enum_name)
        .ok_or_else(|| {
            TypeError::new(format!("unknown enum type '{}' in cast", enum_name))
        })?;

    match &expr.exp {
        Expr::Init { fields } => {
            if fields.len() != 1 {
                return Err(TypeError::new(format!(
                    "enum init requires exactly one variant, got {}",
                    fields.len()
                )));
            }
            let (variant, payload_opt) = &fields[0];
            let member = decl
                .members
                .iter()
                .find(|m| matches!(m, UserTypeMember::EnumVariant { name: n, .. } if n == variant))
                .ok_or_else(|| {
                    TypeError::new(format!(
                        "unknown variant '{}' for enum {}",
                        variant, enum_name
                    ))
                })?;
            match (member, payload_opt) {
                (UserTypeMember::EnumVariant { ty: Some(expected_ty), .. }, Some(payload_expr)) => {
                    let checked = type_check_expr_to_typed(payload_expr, env)?;
                    if !types_compatible(&checked.ty, expected_ty) {
                        return Err(TypeError::new(format!(
                            "variant '{}' expects {:?}, got {:?}",
                            variant, expected_ty, checked.ty
                        )));
                    }
                    Ok(ExprD {
                        exp: Expr::EnumVariant {
                            enum_name: Some(enum_name.to_string()),
                            variant: variant.clone(),
                            payload: Some(Box::new(checked)),
                        },
                        ty: Type::Enum(enum_name.to_string()),
                    })
                }
                (UserTypeMember::EnumVariant { ty: None, .. }, None) => {
                    Ok(ExprD {
                        exp: Expr::EnumVariant {
                            enum_name: Some(enum_name.to_string()),
                            variant: variant.clone(),
                            payload: None,
                        },
                        ty: Type::Enum(enum_name.to_string()),
                    })
                }
                (UserTypeMember::EnumVariant { ty: Some(_), .. }, None) => {
                    Err(TypeError::new(format!(
                        "variant '{}' requires a payload value",
                        variant
                    )))
                }
                (UserTypeMember::EnumVariant { ty: None, .. }, Some(_)) => {
                    Err(TypeError::new(format!(
                        "variant '{}' is a unit variant and takes no payload",
                        variant
                    )))
                }
                _ => unreachable!(),
            }
        }
        other => Err(TypeError::new(format!(
            "enum cast requires {{ .variant [= expr] }}, got {:?}",
            other
        ))),
    }
}

fn type_check_struct_init(
    init: &UncheckedExpr,
    struct_name: &str,
    env: &Environment<Type>,
) -> Result<CheckedExpr, TypeError> {
    match &init.exp {
        Expr::Init { fields } => {
            let decl = env
                .get_type_decl(&UserTypeKind::Struct, struct_name)
                .ok_or_else(|| {
                    TypeError::new(format!("unknown struct type: {}", struct_name))
                })?;

            let mut expected_fields: std::collections::HashSet<String> = decl
                .members
                .iter()
                .filter_map(|m| match m {
                    UserTypeMember::Field(f) => Some(f.name.clone()),
                    _ => None,
                })
                .collect();

            let mut checked_fields = Vec::new();
            for (field_name, field_expr_opt) in fields {
                let field_expr = field_expr_opt.as_ref().ok_or_else(|| {
                    TypeError::new(format!(
                        "field '{}' in struct {} must have a value",
                        field_name, struct_name
                    ))
                })?;

                let field_decl = decl.members.iter().find_map(|m| match m {
                    UserTypeMember::Field(f) if f.name == *field_name => Some(f),
                    _ => None,
                }).ok_or_else(|| {
                    TypeError::new(format!(
                        "unknown field '{}' in struct {}",
                        field_name, struct_name
                    ))
                })?;

                let checked = type_check_expr_to_typed(field_expr, env)?;
                if !types_compatible(&checked.ty, &field_decl.ty) {
                    return Err(TypeError::new(format!(
                        "field '{}' expects {:?}, got {:?}",
                        field_name, field_decl.ty, checked.ty
                    )));
                }
                if !expected_fields.remove(field_name) {
                    return Err(TypeError::new(format!(
                        "duplicate field '{}' in struct {} initializer",
                        field_name, struct_name
                    )));
                }
                checked_fields.push((field_name.clone(), Some(checked)));
            }

            if !expected_fields.is_empty() {
                return Err(TypeError::new(format!(
                    "missing fields in struct {} initializer: {:?}",
                    struct_name,
                    expected_fields.iter().collect::<Vec<_>>()
                )));
            }

            Ok(ExprD {
                exp: Expr::Init {
                    fields: checked_fields,
                },
                ty: Type::Struct(struct_name.to_string()),
            })
        }
        other => Err(TypeError::new(format!(
            "struct declaration requires {{ .field = expr, ... }}, got {:?}",
            other
        ))),
    }
}

fn type_check_enum_init(
    init: &UncheckedExpr,
    enum_name: &str,
    env: &Environment<Type>,
) -> Result<CheckedExpr, TypeError> {
    let inner = match &init.exp {
        Expr::Cast { ty: cast_ty, expr } => {
            let expected = Type::Enum(enum_name.to_string());
            if *cast_ty != expected {
                return Err(TypeError::new(format!(
                    "cast target type {:?} doesn't match declared type {:?}",
                    cast_ty, expected
                )));
            }
            expr
        }
        _ => init,
    };
    match &inner.exp {
        Expr::Init { fields } => {
            if fields.len() != 1 {
                return Err(TypeError::new(format!(
                    "enum init requires exactly one variant, got {}",
                    fields.len()
                )));
            }
            let (variant, payload_opt) = &fields[0];
            let decl = env
                .get_type_decl(&UserTypeKind::Enum, enum_name)
                .ok_or_else(|| {
                    TypeError::new(format!("unknown enum type: {}", enum_name))
                })?;
            let member = decl
                .members
                .iter()
                .find(|m| matches!(m, UserTypeMember::EnumVariant { name: n, .. } if n == variant))
                .ok_or_else(|| {
                    TypeError::new(format!(
                        "unknown variant '{}' for enum {}",
                        variant, enum_name
                    ))
                })?;
            match (member, payload_opt) {
                (UserTypeMember::EnumVariant { ty: Some(expected_ty), .. }, Some(payload_expr)) => {
                    let checked = type_check_expr_to_typed(payload_expr, env)?;
                    if !types_compatible(&checked.ty, expected_ty) {
                        return Err(TypeError::new(format!(
                            "variant '{}' expects {:?} payload, got {:?}",
                            variant, expected_ty, checked.ty
                        )));
                    }
                    Ok(ExprD {
                        exp: Expr::EnumVariant {
                            enum_name: Some(enum_name.to_string()),
                            variant: variant.clone(),
                            payload: Some(Box::new(checked)),
                        },
                        ty: Type::Enum(enum_name.to_string()),
                    })
                }
                (UserTypeMember::EnumVariant { ty: None, .. }, None) => {
                    Ok(ExprD {
                        exp: Expr::EnumVariant {
                            enum_name: Some(enum_name.to_string()),
                            variant: variant.clone(),
                            payload: None,
                        },
                        ty: Type::Enum(enum_name.to_string()),
                    })
                }
                (UserTypeMember::EnumVariant { ty: Some(_), .. }, None) => {
                    Err(TypeError::new(format!(
                        "variant '{}' requires a payload value, use {{ .{} = expr }}",
                        variant, variant
                    )))
                }
                (UserTypeMember::EnumVariant { ty: None, .. }, Some(_)) => {
                    Err(TypeError::new(format!(
                        "variant '{}' is a unit variant and takes no value",
                        variant
                    )))
                }
                _ => unreachable!(),
            }
        }
        other => Err(TypeError::new(format!(
            "enum declaration requires {{ .variant [= expr] }}, got {:?}",
            other
        ))),
    }
}

fn type_check_expr(e: &UncheckedExpr, env: &Environment<Type>) -> Result<Type, TypeError> {
    match &e.exp {
        Expr::Literal(l) => Ok(literal_type(l)),
        Expr::Ident(name) => match env.get(name) {
            Some(Type::Function { .. }) => Err(TypeError::new(format!(
                "cannot use function '{}' as a value",
                name
            ))),
            Some(ty) => Ok(ty.clone()),
            None => Err(TypeError::new(format!("undeclared variable: {}", name))),
        },
        Expr::Neg(inner) => {
            let ty = type_check_expr(inner, env)?;
            if matches!(ty, Type::Int | Type::Float) {
                Ok(ty)
            } else {
                Err(TypeError::new("unary minus requires Int or Float"))
            }
        }
        Expr::Add(l, r) | Expr::Sub(l, r) | Expr::Mul(l, r) | Expr::Div(l, r) => {
            let lt = type_check_expr(l, env)?;
            let rt = type_check_expr(r, env)?;
            numeric_binop_result(&lt, &rt)
        }
        Expr::Eq(l, r) | Expr::Ne(l, r) => {
            let lt = type_check_expr(l, env)?;
            let rt = type_check_expr(r, env)?;
            if !types_compatible(&lt, &rt) {
                return Err(TypeError::new(format!(
                    "equality operands must have compatible types, got {:?} and {:?}",
                    lt, rt
                )));
            }
            Ok(Type::Bool)
        }
        Expr::Lt(l, r) | Expr::Le(l, r) | Expr::Gt(l, r) | Expr::Ge(l, r) => {
            let lt = type_check_expr(l, env)?;
            let rt = type_check_expr(r, env)?;
            if !is_numeric(&lt) || !is_numeric(&rt) {
                return Err(TypeError::new(format!(
                    "ordering comparison requires numeric operands, got {:?} and {:?}",
                    lt, rt
                )));
            }
            Ok(Type::Bool)
        }
        Expr::Not(inner) => {
            let ty = type_check_expr(inner, env)?;
            if ty == Type::Bool {
                Ok(Type::Bool)
            } else {
                Err(TypeError::new("not requires Bool operand"))
            }
        }
        Expr::And(l, r) | Expr::Or(l, r) => {
            let lt = type_check_expr(l, env)?;
            let rt = type_check_expr(r, env)?;
            if lt == Type::Bool && rt == Type::Bool {
                Ok(Type::Bool)
            } else {
                Err(TypeError::new("and/or require Bool operands"))
            }
        }
        Expr::Call { name, args } => {
            let args_checked: Result<Vec<_>, _> = args
                .iter()
                .map(|a| type_check_expr_to_typed(a, env))
                .collect();
            let args_checked = args_checked?;
            match env.get(name) {
                Some(Type::Function {
                    params: param_tys,
                    return_type,
                }) => {
                    if args_checked.len() != param_tys.len() {
                        return Err(TypeError::new(format!(
                            "function '{}' expects {} arguments, got {}",
                            name,
                            param_tys.len(),
                            args_checked.len()
                        )));
                    }
                    for (i, (arg, param_ty)) in
                        args_checked.iter().zip(param_tys.iter()).enumerate()
                    {
                        if !types_compatible(&arg.ty, param_ty) {
                            return Err(TypeError::new(format!(
                                "argument {} to {}: expected {:?}, got {:?}",
                                i + 1,
                                name,
                                param_ty,
                                arg.ty
                            )));
                        }
                    }
                    Ok((**return_type).clone())
                }
                Some(_) => Err(TypeError::new(format!("'{}' is not a function", name))),
                None => Err(TypeError::new(format!("undefined function: {}", name))),
            }
        }
        Expr::ArrayLit(elems) => {
            if elems.is_empty() {
                return Err(TypeError::new("empty array literal needs type annotation"));
            }
            let first = type_check_expr(&elems[0], env)?;
            for e in elems.iter().skip(1) {
                let ty = type_check_expr(e, env)?;
                if !types_compatible(&first, &ty) {
                    return Err(TypeError::new("array elements must have same type"));
                }
            }
            Ok(Type::Array(Box::new(first)))
        }
        Expr::Index { base, index } => {
            let index_ty = type_check_expr(index, env)?;
            if index_ty != Type::Int {
                return Err(TypeError::new("array index must be Int"));
            }
            let base_ty = type_check_expr(base, env)?;
            if let Type::Array(elem) = base_ty {
                Ok(*elem)
            } else {
                Err(TypeError::new("indexed expression must be array"))
            }
        }
        Expr::Member { base, member } => {
            let base_ty = type_check_expr(base, env)?;
            match base_ty {
                Type::Struct(ref identifier) => {
                    let decl = env
                        .get_type_decl(&UserTypeKind::Struct, identifier)
                        .ok_or_else(|| {
                            TypeError::new(format!(
                                "unknown struct type in member access: {}",
                                identifier
                            ))
                        })?;

                    decl.members
                        .iter()
                        .find_map(|m| match m {
                            UserTypeMember::Field(decl) if decl.name == *member => {
                                Some(decl.ty.clone())
                            }
                            _ => None,
                        })
                        .ok_or_else(|| {
                            TypeError::new(format!(
                                "unknown member '{}' on struct {}",
                                member, identifier
                            ))
                        })
                }
                Type::Enum(ref identifier) => Err(TypeError::new(format!(
                    "cannot access enum variants directly, use match for '{}'",
                    identifier
                ))),
                other => Err(TypeError::new(format!(
                    "member access requires struct base type, got {:?}",
                    other
                ))),
            }
        }
        Expr::Init { .. } => Err(TypeError::new(
            "struct/enum init used outside of variable declaration",
        )),
        Expr::Cast { ty, .. } => Ok(ty.clone()),
        Expr::EnumVariant { .. } => Err(TypeError::new(
            "enum variant used outside of cast or declaration",
        )),
    }
}

fn literal_type(l: &Literal) -> Type {
    match l {
        Literal::Int(_) => Type::Int,
        Literal::Float(_) => Type::Float,
        Literal::Str(_) => Type::Str,
        Literal::Bool(_) => Type::Bool,
    }
}

fn numeric_binop_result(l: &Type, r: &Type) -> Result<Type, TypeError> {
    match (l, r) {
        (Type::Int, Type::Int) => Ok(Type::Int),
        (Type::Int, Type::Float) | (Type::Float, Type::Int) | (Type::Float, Type::Float) => {
            Ok(Type::Float)
        }
        _ => Err(TypeError::new("arithmetic operands must be Int or Float")),
    }
}

fn is_numeric(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Float)
}

fn types_compatible(a: &Type, b: &Type) -> bool {
    match (a, b) {
        // Any parameter accepts any argument type.
        (_, Type::Any) => true,
        (Type::Int, Type::Int)
        | (Type::Float, Type::Float)
        | (Type::Bool, Type::Bool)
        | (Type::Str, Type::Str)
        | (Type::Unit, Type::Unit) => true,
        (Type::Int, Type::Float) | (Type::Float, Type::Int) => true,
        (Type::Array(a), Type::Array(b)) => types_compatible(a, b),
        (Type::Struct(a), Type::Struct(b)) => a == b,
        (Type::Enum(a), Type::Enum(b)) => a == b,
        _ => false,
    }
}
