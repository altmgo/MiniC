//! Integration tests for the MiniC TAC code generator.

use mini_c::codegen::tac_code_gen::{translate_program, translate_statement, Environment};
use mini_c::ir::ast::{
    CheckedExpr, CheckedProgram, CheckedStmt, Expr, ExprD, Literal, Statement, StatementD, Type,
    UncheckedProgram,
};
use mini_c::ir::tac::{Address, Instruction, Operator};
use mini_c::parser::program;
use mini_c::semantic::{type_check, TypeError};
use nom::combinator::all_consuming;
use std::path::Path;

// --- Helpers to build type-annotated AST nodes ---

fn int_var(name: &str) -> CheckedExpr {
    ExprD {
        exp: Expr::Ident(name.to_string()),
        ty: Type::Int,
    }
}

fn add(left: CheckedExpr, right: CheckedExpr) -> CheckedExpr {
    ExprD {
        exp: Expr::Add(Box::new(left), Box::new(right)),
        ty: Type::Int,
    }
}

fn lt(left: CheckedExpr, right: CheckedExpr) -> CheckedExpr {
    ExprD {
        exp: Expr::Lt(Box::new(left), Box::new(right)),
        ty: Type::Bool,
    }
}

fn assign(name: &str, value: CheckedExpr) -> CheckedStmt {
    StatementD {
        stmt: Statement::Assign {
            target: Box::new(ExprD {
                exp: Expr::Ident(name.to_string()),
                ty: value.ty.clone(),
            }),
            value: Box::new(value),
        },
        ty: Type::Unit,
    }
}

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn parse_fixture(name: &str) -> UncheckedProgram {
    let source = std::fs::read_to_string(fixtures_dir().join(name)).expect("fixture should exist");
    let result = all_consuming(program)(source.trim())
        .map_err(|e| e.map_input(String::from))
        .expect("fixture should parse");
    result.1
}

fn type_check_fixture(name: &str) -> Result<CheckedProgram, TypeError> {
    type_check(&parse_fixture(name))
}

// Fixture: if (x < y) { z = x + y; } else { z = x; }
//
// Expected TAC:
//   if x >= y goto Label1:    <- negated relational jumps to else
//   temp1 = x + y
//   z = temp1
//   goto Label2:
//   Label1:
//   z = x
//   Label2:
#[test]
fn test_if_else_with_relational_condition() {
    let stmt = StatementD {
        stmt: Statement::If {
            cond: Box::new(lt(int_var("x"), int_var("y"))),
            then_branch: Box::new(assign("z", add(int_var("x"), int_var("y")))),
            else_branch: Some(Box::new(assign("z", int_var("x")))),
        },
        ty: Type::Unit,
    };

    let mut env = Environment::new();
    let instructions = translate_statement(stmt, &mut env);

    let x = Address::Variable("x".to_string(), Type::Int);
    let y = Address::Variable("y".to_string(), Type::Int);
    let z = Address::Variable("z".to_string(), Type::Int);
    let temp = Address::Temporary("temp1".to_string(), Type::Int);

    assert_eq!(
        instructions,
        vec![
            Instruction::ConditionalJMPRelational(
                Operator::GTE,
                x.clone(),
                y.clone(),
                "Label1:".to_string()
            ),
            Instruction::BinaryAssignment(Operator::Add, temp.clone(), x.clone(), y.clone()),
            Instruction::CopyAssignment(z.clone(), temp),
            Instruction::JMP("Label2:".to_string()),
            Instruction::Label("Label1:".to_string()),
            Instruction::CopyAssignment(z, x),
            Instruction::Label("Label2:".to_string()),
        ]
    );
}

#[test]
fn test_aggregate_types_fixture_generates_tac() {
    let checked =
        type_check_fixture("tac_types.minic").expect("tac fixture should type-check");

    let mut env = Environment::new();
    let instructions = translate_program(checked, &mut env);

    assert_eq!(
        instructions,
        vec![
            Instruction::Label("main".to_string()),
            // struct Point p = { .valid = true, .x = 42 }
            Instruction::CopyAssignment(
                Address::Variable("p.valid".to_string(), Type::Bool),
                Address::Constant(Literal::Bool(true), Type::Bool),
            ),
            Instruction::CopyAssignment(
                Address::Variable("p.x".to_string(), Type::Int),
                Address::Constant(Literal::Int(42), Type::Int),
            ),
            // print(p.valid)
            Instruction::Param(Address::Variable("p.valid".to_string(), Type::Bool)),
            Instruction::Call(None, "print".to_string(), 1),
            // enum Kind k = { .B = 42 }
            Instruction::CopyAssignment(
                Address::Variable("k.tag".to_string(), Type::Int),
                Address::Constant(Literal::Int(0), Type::Int),
            ),
            Instruction::CopyAssignment(
                Address::Variable("k.payload".to_string(), Type::Int),
                Address::Constant(Literal::Int(42), Type::Int),
            ),
            // match k: case B (ordinal 0) -> if tag != 0, skip to Label2
            Instruction::ConditionalJMPRelational(
                Operator::NE,
                Address::Variable("k.tag".to_string(), Type::Int),
                Address::Constant(Literal::Int(0), Type::Int),
                "Label2:".to_string(),
            ),
            // bind B = k.payload
            Instruction::CopyAssignment(
                Address::Variable("B".to_string(), Type::Int),
                Address::Variable("k.payload".to_string(), Type::Int),
            ),
            // print(B)
            Instruction::Param(Address::Variable("B".to_string(), Type::Int)),
            Instruction::Call(None, "print".to_string(), 1),
            Instruction::JMP("Label1:".to_string()),
            // case A (ordinal 1, last arm, fallthrough)
            Instruction::Label("Label2:".to_string()),
            // print(0)
            Instruction::Param(Address::Constant(Literal::Int(0), Type::Int)),
            Instruction::Call(None, "print".to_string(), 1),
            // end (last arm falls through)
            Instruction::Label("Label1:".to_string()),
        ]
    );
}

#[test]
fn test_aggregate_struct_member_fixture_generates_tac() {
    let checked = type_check_fixture("aggregate_struct_member_success.minic")
        .expect("aggregate struct member fixture should type-check");

    let mut env = Environment::new();
    let instructions = translate_program(checked, &mut env);

    assert_eq!(
        instructions,
        vec![
            Instruction::Label("main".to_string()),
            // struct Flag flag = { .enabled = true }
            Instruction::CopyAssignment(
                Address::Variable("flag.enabled".to_string(), Type::Bool),
                Address::Constant(Literal::Bool(true), Type::Bool),
            ),
            // print(flag.enabled)
            Instruction::Param(Address::Variable("flag.enabled".to_string(), Type::Bool)),
            Instruction::Call(None, "print".to_string(), 1),
        ]
    );
}

#[test]
fn test_aggregate_unknown_struct_fixture_fails_type_check() {
    let err = type_check_fixture("aggregate_unknown_struct_fail.minic")
        .expect_err("unknown struct fixture should fail type-checking");

    assert!(
        err.message.contains("unknown struct type"),
        "expected unknown struct type error, got: {}",
        err.message
    );
    assert!(
        err.message.contains("Missing"),
        "expected missing struct name in error, got: {}",
        err.message
    );
}

#[test]
fn test_nested_enum_in_struct_field_init() {
    let checked = type_check_fixture("nested_enum_in_struct.minic")
        .expect("nested enum fixture should type-check");

    let mut env = Environment::new();
    let instructions = translate_program(checked, &mut env);

    assert_eq!(
        instructions,
        vec![
            Instruction::Label("main".to_string()),
            // struct Outer o = { .field = (enum Inner){ .V = 42 } }
            // V is first variant, ordinal 0
            Instruction::CopyAssignment(
                Address::Variable("o.field.tag".to_string(), Type::Int),
                Address::Constant(Literal::Int(0), Type::Int),
            ),
            Instruction::CopyAssignment(
                Address::Variable("o.field.payload".to_string(), Type::Int),
                Address::Constant(Literal::Int(42), Type::Int),
            ),
        ]
    );
}
