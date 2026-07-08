//! Integration tests for the MiniC type checker.

use mini_c::ir::ast::{CheckedProgram, Type};
use mini_c::parser::program;
use mini_c::semantic::type_check;
use nom::combinator::all_consuming;

fn parse_and_type_check(src: &str) -> Result<CheckedProgram, mini_c::semantic::TypeError> {
    let (_, prog) = all_consuming(program)(src).map_err(|_| mini_c::semantic::TypeError {
        message: "parse failed".to_string(),
    })?;
    type_check(&prog)
}

#[test]
fn test_type_check_simple_assign() {
    let result = parse_and_type_check("void main() int x = 1;");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_int_float_coercion() {
    let result = parse_and_type_check("void main() float x = 1 + 3.14;");
    assert!(result.is_ok());
    let prog = result.unwrap();
    let main_fn = prog.functions.iter().find(|f| f.name == "main").unwrap();
    if let mini_c::ir::ast::Statement::Decl { ref init, .. } = main_fn.body.stmt {
        assert_eq!(init.ty, Type::Float);
    } else {
        panic!("expected Decl");
    }
}

#[test]
fn test_type_check_undeclared_var() {
    let result = parse_and_type_check("void main() x = y;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("undeclared"));
}

#[test]
fn test_type_check_bool_condition() {
    let result = parse_and_type_check("void main() if true { int x = 1; }");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_array_literal() {
    let result = parse_and_type_check("void main() int[] x = [1, 2, 3];");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_array_index() {
    let result = parse_and_type_check("void main() { int[] arr = [1, 2]; int x = arr[0]; }");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_call_arg_type_mismatch() {
    let result = parse_and_type_check("void foo(int x) x = x;\nvoid main() foo(true);");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("argument"));
}

#[test]
fn test_type_check_missing_main() {
    let result = parse_and_type_check("void foo() int x = 1;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("main"));
}

#[test]
fn test_type_check_decl_then_assign() {
    let result = parse_and_type_check("void main() { int x = 1; x = 2; }");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_assign_undeclared() {
    let result = parse_and_type_check("void main() x = 1;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("undeclared"));
}

#[test]
fn test_type_check_redeclaration() {
    let result = parse_and_type_check("void main() { int x = 1; int x = 2; }");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("redeclaration"));
}

#[test]
fn test_type_check_relational_type_mismatch() {
    let result = parse_and_type_check("void main() bool r = \"hello\" == 42;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("compatible"));
}

#[test]
fn test_type_check_ordering_requires_numeric() {
    let result = parse_and_type_check("void main() bool r = true < false;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("numeric"));
}

#[test]
fn test_type_check_equality_same_type_ok() {
    let result = parse_and_type_check("void main() bool r = 1 == 2;");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_main_non_void_return() {
    let result = parse_and_type_check("int main() int x = 1;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("void"));
}

#[test]
fn test_type_check_main_with_params() {
    let result = parse_and_type_check("void main(int x) int y = 1;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("parameters"));
}

#[test]
fn test_type_check_return_void_ok() {
    let result = parse_and_type_check("void main() { int x = 1; return; }");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_return_value_in_void_fn() {
    let result = parse_and_type_check("void main() return 1;");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("void function must not return a value"));
}

#[test]
fn test_type_check_return_correct_type() {
    let result = parse_and_type_check("int foo() return 42;\nvoid main() int x = 1;");
    assert!(result.is_ok());
}

#[test]
fn test_type_check_return_wrong_type() {
    let result = parse_and_type_check("int foo() return true;\nvoid main() int x = 1;");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("return type mismatch"));
}

#[test]
fn test_type_check_block_scoping() {
    // variable declared inside a block should not be visible after the block
    let result = parse_and_type_check("void main() { { int x = 1; } x = 2; }");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("undeclared"));
}

// ---------------------------------------------------------------------------
// 7.2 print accepts any type via Type::Any
// ---------------------------------------------------------------------------
#[test]
fn test_type_check_print_accepts_int() {
    assert!(parse_and_type_check("void main() { print(42); }").is_ok());
}

#[test]
fn test_type_check_print_accepts_bool() {
    assert!(parse_and_type_check("void main() { print(true); }").is_ok());
}

#[test]
fn test_type_check_print_accepts_str() {
    assert!(parse_and_type_check("void main() { print(\"hello\"); }").is_ok());
}

#[test]
fn test_type_check_print_accepts_float() {
    assert!(parse_and_type_check("void main() { print(3.14); }").is_ok());
}

#[test]
fn test_type_check_print_accepts_array() {
    assert!(parse_and_type_check("void main() { print([1, 2, 3]); }").is_ok());
}

// ---------------------------------------------------------------------------
// 7.3 Wrong arity on stdlib function reports type error
// ---------------------------------------------------------------------------
#[test]
fn test_type_check_sqrt_wrong_arity() {
    let result = parse_and_type_check("void main() { float r = sqrt(); }");
    assert!(result.is_err(), "expected arity error for sqrt()");
}

#[test]
fn test_type_check_print_wrong_arity() {
    let result = parse_and_type_check("void main() { print(1, 2); }");
    assert!(result.is_err(), "expected arity error for print(1, 2)");
}

// ---------------------------------------------------------------------------
// Aggregate Types
// ---------------------------------------------------------------------------

#[test]
fn test_type_check_accepts_struct_decl_and_member_access() {
    let result = parse_and_type_check(
        "struct Point { int x; int y; }\nvoid main() { struct Point p = { .x = 12, .y = 0 }; int v = p.x; }",
    );
    assert!(result.is_ok());
}

#[test]
fn test_type_check_accepts_enum_decl_and_init() {
    let result = parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .Some = 42 }; }",
    );
    assert!(result.is_ok());
}

#[test]
fn test_type_check_accepts_match_on_enum() {
    let result = parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .Some = 42 }; match x { case Some: { int y = Some; } case None: { int z = 0; } } }",
    );
    assert!(result.is_ok());
}

#[test]
fn test_type_check_rejects_unknown_struct_member_access() {
    let result = parse_and_type_check(
        "struct Point { int x; }\nvoid main() { struct Point p = { .x = 0 }; int v = p.y; }",
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("unknown member"));
}

#[test]
fn test_type_check_rejects_enum_member_access() {
    let result = parse_and_type_check(
        "enum Color { Red; Green; }\nvoid main() { enum Color c = { .Red }; int v = c.Red; }",
    );
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("cannot access enum variants directly"));
}

#[test]
fn test_type_check_rejects_unknown_type_declaration_use() {
    let result = parse_and_type_check("void main() { struct Missing x = { .x = 0 }; }");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("unknown struct type"));
}

#[test]
fn test_type_check_rejects_member_access_on_non_struct_value() {
    let result = parse_and_type_check("void main() { int x = 0; int y = x.foo; }");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("member access requires struct base type"));
}

#[test]
fn test_type_check_rejects_duplicate_type_declarations() {
    let result = parse_and_type_check(
        "struct Point { int x; }\nstruct Point { int y; }\nvoid main() { int z = 0; }",
    );
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("duplicate type declaration"));
}

#[test]
fn test_type_check_rejects_duplicate_struct_init_fields() {
    let result = parse_and_type_check(
        "struct Point { int x; int y; }\nvoid main() { struct Point p = { .x = 1, .x = 2, .y = 3 }; }",
    );
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("duplicate field"));
}

#[test]
fn test_type_check_rejects_cast_mismatch_in_enum_decl() {
    let result = parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = (int){ .Some = 42 }; }",
    );
    assert!(result.is_err());
}

#[test]
fn test_type_check_rejects_unit_variant_with_payload_in_decl() {
    let result = parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .None = 42 }; }",
    );
    assert!(result.is_err());
}

#[test]
fn test_type_check_rejects_payload_variant_without_arg_in_decl() {
    let result = parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .Some }; }",
    );
    assert!(result.is_err());
}

#[test]
fn test_type_check_accepts_cast_in_expression() {
    let result = parse_and_type_check(
        "enum Option { int Some; None; }\nvoid foo(enum Option x) { }\nvoid main() { foo((enum Option){ .Some = 42 }); }",
    );
    assert!(result.is_ok());
}
