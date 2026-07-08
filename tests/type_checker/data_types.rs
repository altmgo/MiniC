use super::helpers::parse_and_type_check;

#[test]
fn struct_decl_and_member_access() {
    assert!(parse_and_type_check(
        "struct Point { int x; int y; }\nvoid main() { struct Point p = { .x = 12, .y = 0 }; int v = p.x; }",
    ).is_ok());
}

#[test]
fn enum_decl_and_init() {
    assert!(parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .Some = 42 }; }",
    )
    .is_ok());
}

#[test]
fn match_on_enum() {
    assert!(parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .Some = 42 }; match x { case Some: { int y = Some; } case None: { int z = 0; } } }",
    ).is_ok());
}

#[test]
fn struct_unknown_member() {
    let result = parse_and_type_check(
        "struct Point { int x; }\nvoid main() { struct Point p = { .x = 0 }; int v = p.y; }",
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("unknown member"));
}

#[test]
fn enum_member_access() {
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
fn unknown_type_declaration_use() {
    let result = parse_and_type_check("void main() { struct Missing x = { .x = 0 }; }");
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("unknown struct type"));
}

#[test]
fn member_access_on_non_struct() {
    let result = parse_and_type_check("void main() { int x = 0; int y = x.foo; }");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .message
        .contains("member access requires struct base type"));
}

#[test]
fn duplicate_type_declarations() {
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
fn duplicate_struct_init_fields() {
    let result = parse_and_type_check(
        "struct Point { int x; int y; }\nvoid main() { struct Point p = { .x = 1, .x = 2, .y = 3 }; }",
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().message.contains("duplicate field"));
}

#[test]
fn cast_mismatch_in_enum_decl() {
    assert!(parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = (int){ .Some = 42 }; }",
    )
    .is_err());
}

#[test]
fn unit_variant_with_payload() {
    assert!(parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .None = 42 }; }",
    )
    .is_err());
}

#[test]
fn payload_variant_without_arg() {
    assert!(parse_and_type_check(
        "enum Option { int Some; None; }\nvoid main() { enum Option x = { .Some }; }",
    )
    .is_err());
}

#[test]
fn cast_in_expression() {
    assert!(parse_and_type_check(
        "enum Option { int Some; None; }\nvoid foo(enum Option x) { }\nvoid main() { foo((enum Option){ .Some = 42 }); }",
    ).is_ok());
}

#[test]
fn nested_struct_init() {
    assert!(parse_and_type_check(
        "struct Inner { int x; }\nstruct Outer { struct Inner inner; }\nvoid main() { struct Outer o = { .inner = { .x = 42 } }; }",
    ).is_ok());
}

#[test]
fn struct_init_in_call() {
    assert!(parse_and_type_check(
        "struct Point { int x; int y; }\nvoid foo(struct Point p) { }\nvoid main() { foo({ .x = 1, .y = 2 }); }",
    ).is_ok());
}

#[test]
fn enum_init_in_struct_field() {
    assert!(parse_and_type_check(
        "enum Inner { int V; None; }\nstruct Outer { enum Inner field; }\nvoid main() { struct Outer o = { .field = { .V = 42 } }; }",
    ).is_ok());
}

#[test]
fn enum_init_in_call() {
    assert!(parse_and_type_check(
        "enum Option { int Some; None; }\nvoid foo(enum Option x) { }\nvoid main() { foo({ .Some = 42 }); }",
    ).is_ok());
}

#[test]
fn struct_init_in_call_type_mismatch() {
    assert!(parse_and_type_check(
        "struct Point { int x; }\nstruct Other { int y; }\nvoid foo(struct Point p) { }\nvoid main() { foo({ .y = 1 }); }",
    ).is_err());
}

#[test]
fn enum_init_in_call_type_mismatch() {
    assert!(parse_and_type_check(
        "enum A { int X; }\nenum B { int Y; }\nvoid foo(enum A a) { }\nvoid main() { foo({ .Y = 1 }); }",
    ).is_err());
}
