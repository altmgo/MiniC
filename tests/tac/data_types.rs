use mini_c::codegen::tac_code_gen::{translate_program, Environment};
use mini_c::ir::ast::{CheckedProgram, Literal, Type, UncheckedProgram};
use mini_c::ir::tac::{Address, Instruction, Operator};
use mini_c::parser::program;
use mini_c::semantic::type_check;
use nom::combinator::all_consuming;
use std::path::Path;

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

fn type_check_fixture(name: &str) -> Result<CheckedProgram, mini_c::semantic::TypeError> {
    type_check(&parse_fixture(name))
}

#[test]
fn struct_init_tac() {
    let checked = type_check_fixture("tac_struct.minic").expect("struct fixture should type-check");
    let mut env = Environment::new();
    let instructions = translate_program(checked, &mut env);

    assert_eq!(
        instructions,
        vec![
            Instruction::Label("main".to_string()),
            Instruction::CopyAssignment(
                Address::Variable("p.valid".to_string(), Type::Bool),
                Address::Constant(Literal::Bool(true), Type::Bool)
            ),
            Instruction::CopyAssignment(
                Address::Variable("p.x".to_string(), Type::Int),
                Address::Constant(Literal::Int(42), Type::Int)
            ),
            Instruction::Param(Address::Variable("p.valid".to_string(), Type::Bool)),
            Instruction::Call(None, "print".to_string(), 1),
        ]
    );
}

#[test]
fn enum_init_match_tac() {
    let checked = type_check_fixture("tac_enum.minic").expect("enum fixture should type-check");
    let mut env = Environment::new();
    let instructions = translate_program(checked, &mut env);

    assert_eq!(
        instructions,
        vec![
            Instruction::Label("main".to_string()),
            Instruction::CopyAssignment(
                Address::Variable("k.tag".to_string(), Type::Int),
                Address::Constant(Literal::Int(0), Type::Int)
            ),
            Instruction::CopyAssignment(
                Address::Variable("k.payload".to_string(), Type::Int),
                Address::Constant(Literal::Int(42), Type::Int)
            ),
            Instruction::ConditionalJMPRelational(
                Operator::NE,
                Address::Variable("k.tag".to_string(), Type::Int),
                Address::Constant(Literal::Int(0), Type::Int),
                "Label2:".to_string()
            ),
            Instruction::CopyAssignment(
                Address::Variable("B".to_string(), Type::Int),
                Address::Variable("k.payload".to_string(), Type::Int)
            ),
            Instruction::Param(Address::Variable("B".to_string(), Type::Int)),
            Instruction::Call(None, "print".to_string(), 1),
            Instruction::JMP("Label1:".to_string()),
            Instruction::Label("Label2:".to_string()),
            Instruction::Param(Address::Constant(Literal::Int(0), Type::Int)),
            Instruction::Call(None, "print".to_string(), 1),
            Instruction::Label("Label1:".to_string()),
        ]
    );
}

#[test]
fn nested_types_tac() {
    let checked = type_check_fixture("tac_nested.minic").expect("nested fixture should type-check");
    let mut env = Environment::new();
    let instructions = translate_program(checked, &mut env);

    assert_eq!(
        instructions,
        vec![
            Instruction::Label("main".to_string()),
            Instruction::CopyAssignment(
                Address::Variable("o.field.tag".to_string(), Type::Int),
                Address::Constant(Literal::Int(0), Type::Int)
            ),
            Instruction::CopyAssignment(
                Address::Variable("o.field.payload".to_string(), Type::Int),
                Address::Constant(Literal::Int(42), Type::Int)
            ),
        ]
    );
}
