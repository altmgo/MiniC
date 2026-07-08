use crate::ir::ast::{
    CheckedExpr, CheckedFunDecl, CheckedProgram, CheckedStmt, Expr, ExprD, Literal, Statement,
    Type, UDTDecl, UDTKind, UDTMember,
};
use crate::ir::tac::{Address, Instruction, Operator, TACProgram};

#[derive(Clone)]
pub struct Environment {
    current_label: usize,
    current_temporary: usize,
    current_struct_temp: usize,
    type_declarations: Vec<UDTDecl>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            current_label: 0,
            current_temporary: 0,
            current_struct_temp: 0,
            type_declarations: Vec::new(),
        }
    }

    fn register_type_declarations(&mut self, type_declarations: Vec<UDTDecl>) {
        self.type_declarations = type_declarations;
    }

    fn new_label(&mut self) -> String {
        self.current_label += 1;
        format!("Label{}:", self.current_label)
    }

    fn new_temporary(&mut self) -> String {
        self.current_temporary += 1;
        format!("temp{}", self.current_temporary)
    }

    fn new_struct_temp(&mut self) -> String {
        self.current_struct_temp += 1;
        format!("_init{}", self.current_struct_temp)
    }

    fn variant_tag_index(&self, enum_name: &str, variant: &str) -> i64 {
        let decl = self
            .type_declarations
            .iter()
            .find(|decl| decl.specifier == UDTKind::Enum && decl.identifier == enum_name)
            .unwrap_or_else(|| unreachable!("checked enum type must be declared"));

        for (i, member) in decl.members.iter().enumerate() {
            if let UDTMember::EnumVariant { name, .. } = member {
                if name == variant {
                    return i as i64;
                }
            }
        }

        unreachable!("checked enum variant must exist")
    }

    fn variant_payload_ty(&self, enum_name: &str, variant: &str) -> Option<Type> {
        self.type_declarations
            .iter()
            .find(|d| d.specifier == UDTKind::Enum && d.identifier == enum_name)?
            .members
            .iter()
            .find_map(|m| match m {
                UDTMember::EnumVariant { name, ty } if name == variant => ty.clone(),
                _ => None,
            })
    }
}

pub fn translate_program(program: CheckedProgram, env: &mut Environment) -> TACProgram {
    env.register_type_declarations(program.type_declarations.clone());
    let main_fn = program.main_function();
    match main_fn {
        None => unreachable!("[Impossible] program must have a main function"),
        Some(f) => translate_function(f.clone(), env),
    }
}

fn translate_function(function: CheckedFunDecl, env: &mut Environment) -> TACProgram {
    let mut instructions = if let Statement::Block { seq: stmts } = function.body.stmt {
        stmts
            .into_iter()
            .flat_map(|stmt| translate_statement(stmt, env))
            .collect::<Vec<_>>()
    } else {
        translate_statement(*(function.body), env)
    };
    instructions.insert(0, Instruction::Label(function.name.clone()));
    instructions
}

pub fn translate_statement(statement: CheckedStmt, env: &mut Environment) -> Vec<Instruction> {
    let mut res: Vec<Instruction> = Vec::new();

    match statement.stmt {
        Statement::Block { seq } => seq
            .into_iter()
            .flat_map(|s| translate_statement(s, env))
            .collect::<Vec<_>>(),
        Statement::Decl { name, ty, init } => {
            let init_box = *init;
            match init_box.exp {
                Expr::Init { fields } => {
                    let mut res = Vec::new();
                    for (field_name, field_expr_opt) in fields {
                        let field_expr = field_expr_opt.expect("struct field must have a value");

                        // Handle enum init inside struct field init.
                        // After type checking, the field value may be:
                        //   Cast { ty: Enum(_), expr: EnumVariant { .. }  (from explicit cast)
                        //   EnumVariant { .. }                           (from bare init)
                        let is_enum_field = if let Expr::Cast {
                            ty: Type::Enum(_),
                            expr: cast_inner,
                        } = &field_expr.exp
                        {
                            matches!(&cast_inner.exp, Expr::EnumVariant { .. })
                        } else {
                            matches!(&field_expr.exp, Expr::EnumVariant { .. })
                        };

                        if is_enum_field {
                            let variant_expr = match &field_expr.exp {
                                Expr::Cast {
                                    expr: cast_inner, ..
                                } => *cast_inner.clone(),
                                _ => field_expr.clone(),
                            };
                            if let Expr::EnumVariant {
                                enum_name,
                                variant,
                                payload,
                            } = variant_expr.exp
                            {
                                let enum_name = enum_name.unwrap_or_default();
                                let ordinal = env.variant_tag_index(&enum_name, &variant);
                                res.push(Instruction::CopyAssignment(
                                    Address::Variable(
                                        format!("{}.{}.tag", name, field_name),
                                        Type::Int,
                                    ),
                                    Address::Constant(Literal::Int(ordinal), Type::Int),
                                ));
                                if let Some(payload_expr) = payload {
                                    let payload_ty = payload_expr.ty.clone();
                                    let (addr, insts) = translate_expression(*payload_expr, env);
                                    res.extend(insts);
                                    res.push(Instruction::CopyAssignment(
                                        Address::Variable(
                                            format!("{}.{}.payload", name, field_name),
                                            payload_ty,
                                        ),
                                        addr,
                                    ));
                                }
                                continue;
                            }
                        }

                        // Handle nested struct init: { .field = { .inner = val } }
                        if let Expr::Init {
                            fields: inner_fields,
                        } = field_expr.exp
                        {
                            for (inner_name, inner_opt) in inner_fields {
                                let inner_expr = inner_opt.expect("struct field must have a value");
                                let inner_ty = inner_expr.ty.clone();
                                let (addr, insts) = translate_expression(inner_expr, env);
                                res.extend(insts);
                                res.push(Instruction::CopyAssignment(
                                    Address::Variable(
                                        format!("{}.{}.{}", name, field_name, inner_name),
                                        inner_ty,
                                    ),
                                    addr,
                                ));
                            }
                            continue;
                        }

                        let field_ty = field_expr.ty.clone();
                        let (addr, insts) = translate_expression(field_expr, env);
                        res.extend(insts);
                        res.push(Instruction::CopyAssignment(
                            Address::Variable(format!("{}.{}", name, field_name), field_ty),
                            addr,
                        ));
                    }
                    res
                }
                Expr::EnumVariant {
                    enum_name,
                    variant,
                    payload,
                } => {
                    let enum_name = enum_name.unwrap_or_default();
                    let ordinal = env.variant_tag_index(&enum_name, &variant);
                    let mut res = vec![Instruction::CopyAssignment(
                        Address::Variable(format!("{}.tag", name), Type::Int),
                        Address::Constant(Literal::Int(ordinal), Type::Int),
                    )];
                    if let Some(payload_expr) = payload {
                        let payload_ty = payload_expr.ty.clone();
                        let (addr, insts) = translate_expression(*payload_expr, env);
                        res.extend(insts);
                        res.push(Instruction::CopyAssignment(
                            Address::Variable(format!("{}.payload", name), payload_ty),
                            addr,
                        ));
                    }
                    res
                }
                other => {
                    let (expression_address, instructions) = translate_expression(
                        ExprD {
                            exp: other,
                            ty: init_box.ty,
                        },
                        env,
                    );
                    let mut res = instructions;
                    res.push(Instruction::CopyAssignment(
                        Address::Variable(name, ty),
                        expression_address,
                    ));
                    res
                }
            }
        }
        Statement::Assign { target, value } => {
            let var_address = translate_lvalue(*target, env);
            let (expression_address, instructions) = translate_expression(*value, env);
            res.extend(instructions);
            res.push(Instruction::CopyAssignment(var_address, expression_address));
            res
        }
        Statement::Call { name, args } => {
            // addresses_and_instructions :: [(Address, [Instruction])]
            let addresses_and_instructions = args
                .into_iter()
                .map(|expr| translate_expression(expr, env))
                .collect::<Vec<_>>();
            let mut instructions =
                addresses_and_instructions
                    .iter()
                    .fold(vec![], |mut acc, (_, inst)| {
                        acc.extend(inst.clone());
                        acc
                    });

            // includes a 'param' instruction to the
            // every addresses built from the arguments.
            for (addr, _) in &addresses_and_instructions {
                instructions.push(Instruction::Param(addr.clone()));
            }
            instructions.push(Instruction::Call(
                None,
                name,
                addresses_and_instructions.len(),
            ));
            instructions
        }
        Statement::If {
            cond,
            then_branch: then_body,
            else_branch: Some(else_body),
        } => {
            let label_else = env.new_label();
            let label_end_if = env.new_label();
            let mut instructions = translate_conditional_false(*cond, env, label_else.clone());
            instructions.extend(translate_statement(*then_body, env));
            instructions.push(Instruction::JMP(label_end_if.clone()));
            instructions.push(Instruction::Label(label_else));
            instructions.extend(translate_statement(*else_body, env));
            instructions.push(Instruction::Label(label_end_if));
            instructions
        }
        Statement::Match { target, arms } => {
            let mut res = Vec::new();
            let end_label = env.new_label();

            let enum_name = match &target.ty {
                Type::Enum(name) => name.clone(),
                _ => unreachable!("match target must be enum type"),
            };

            let (target_addr, target_insts) = translate_expression(*target, env);
            res.extend(target_insts);

            let tag_var = match &target_addr {
                Address::Variable(name, _) => name.clone(),
                _ => unreachable!("match target must be a variable"),
            };

            let num_arms = arms.len();
            for (i, arm) in arms.into_iter().enumerate() {
                let is_last = i == num_arms - 1;
                let next_label = if !is_last {
                    Some(env.new_label())
                } else {
                    None
                };

                let ordinal = env.variant_tag_index(&enum_name, &arm.variant);

                let tag_addr = Address::Variable(format!("{}.tag", tag_var), Type::Int);
                let ordinal_addr = Address::Constant(Literal::Int(ordinal), Type::Int);

                if let Some(ref label) = next_label {
                    res.push(Instruction::ConditionalJMPRelational(
                        Operator::NE,
                        tag_addr,
                        ordinal_addr,
                        label.clone(),
                    ));
                }

                if let Some(binding) = arm.binding {
                    let payload_ty = env
                        .variant_payload_ty(&enum_name, &arm.variant)
                        .expect("variant with binding must have payload type");
                    let payload_addr =
                        Address::Variable(format!("{}.payload", tag_var), payload_ty.clone());
                    res.push(Instruction::CopyAssignment(
                        Address::Variable(binding, payload_ty),
                        payload_addr,
                    ));
                }

                res.extend(translate_statement(*arm.body, env));
                if !is_last {
                    res.push(Instruction::JMP(end_label.clone()));
                }

                if let Some(label) = next_label {
                    res.push(Instruction::Label(label));
                }
            }

            res.push(Instruction::Label(end_label));
            res
        }
        _ => todo!(),
    }
}

fn translate_lvalue(target: CheckedExpr, env: &mut Environment) -> Address {
    match target.exp {
        Expr::Ident(name) => Address::Variable(name, target.ty),
        Expr::Member { base, member } => translate_member_address(*base, member, target.ty, env),
        _ => todo!(),
    }
}

fn translate_member_address(
    base: CheckedExpr,
    member: String,
    member_ty: Type,
    _env: &mut Environment,
) -> Address {
    Address::Variable(format!("{}.{}", member_base_name(base), member), member_ty)
}

fn member_base_name(base: CheckedExpr) -> String {
    match base.exp {
        Expr::Ident(name) => name,
        Expr::Member {
            base: nested_base,
            member,
        } => format!("{}.{}", member_base_name(*nested_base), member),
        _ => todo!(),
    }
}

fn translate_conditional_false(
    expression: CheckedExpr,
    env: &mut Environment,
    false_label: String,
) -> Vec<Instruction> {
    match expression.exp {
        Expr::Literal(Literal::Bool(true)) => vec![],
        Expr::Literal(Literal::Bool(false)) => vec![Instruction::JMP(false_label)],
        Expr::Ident(name) => {
            let addr = Address::Variable(name.to_string(), expression.ty);
            vec![Instruction::ConditionalJMPFalse(addr, false_label)]
        }
        Expr::Lt(left, right) => {
            translate_relational_false(*left, *right, Operator::GTE, false_label, env)
        }
        Expr::Le(left, right) => {
            translate_relational_false(*left, *right, Operator::GT, false_label, env)
        }
        Expr::Gt(left, right) => {
            translate_relational_false(*left, *right, Operator::LTE, false_label, env)
        }
        Expr::Ge(left, right) => {
            translate_relational_false(*left, *right, Operator::LT, false_label, env)
        }
        Expr::Eq(left, right) => {
            translate_relational_false(*left, *right, Operator::NE, false_label, env)
        }
        Expr::Ne(left, right) => {
            translate_relational_false(*left, *right, Operator::EQ, false_label, env)
        }
        _ => {
            let (addr, mut instructions) = translate_expression(
                ExprD {
                    exp: expression.exp,
                    ty: expression.ty,
                },
                env,
            );
            instructions.push(Instruction::ConditionalJMPFalse(addr, false_label));
            instructions
        }
    }
}

fn translate_expression(
    expression: CheckedExpr,
    env: &mut Environment,
) -> (Address, Vec<Instruction>) {
    match expression.exp {
        Expr::Literal(value) => (Address::Constant(value, expression.ty), vec![]),
        Expr::Ident(name) => (Address::Variable(name.to_string(), expression.ty), vec![]),
        Expr::Member { base, member } => (
            translate_member_address(*base, member, expression.ty, env),
            vec![],
        ),
        // Boolean Expressions. 'and' and 'or' implement a short circuit semantics.
        Expr::Not(exp) => {
            let (addr, mut instructions) = translate_expression(*exp, env);
            let label_false = env.new_label();
            let label_exit = env.new_label();
            let temp = Address::Temporary(env.new_temporary(), Type::Bool);
            instructions.push(Instruction::ConditionalJMPFalse(addr, label_false.clone()));
            instructions.push(Instruction::CopyAssignment(
                temp.clone(),
                Address::Constant(Literal::Bool(false), Type::Bool),
            ));
            instructions.push(Instruction::JMP(label_exit.clone()));
            instructions.push(Instruction::Label(label_false));
            instructions.push(Instruction::CopyAssignment(
                temp.clone(),
                Address::Constant(Literal::Bool(true), Type::Bool),
            ));
            instructions.push(Instruction::Label(label_exit));
            (temp, instructions)
        }
        Expr::Or(left, right) => {
            let (l_addr, l_instructions) = translate_expression(*left, env);
            let (r_addr, r_instructions) = translate_expression(*right, env);
            let label_true = env.new_label();
            let label_false = env.new_label();
            let label_exit = env.new_label();
            let temp = Address::Temporary(env.new_temporary(), Type::Bool);
            let mut instructions = l_instructions;
            instructions.push(Instruction::ConditionalJMPFalse(
                l_addr,
                label_false.clone(),
            ));
            instructions.push(Instruction::JMP(label_true.clone()));
            instructions.push(Instruction::Label(label_false));
            instructions.extend(r_instructions);
            instructions.push(Instruction::ConditionalJMP(r_addr, label_true.clone()));
            instructions.push(Instruction::CopyAssignment(
                temp.clone(),
                Address::Constant(Literal::Bool(false), Type::Bool),
            ));
            instructions.push(Instruction::JMP(label_exit.clone()));
            instructions.push(Instruction::Label(label_true));
            instructions.push(Instruction::CopyAssignment(
                temp.clone(),
                Address::Constant(Literal::Bool(true), Type::Bool),
            ));
            instructions.push(Instruction::Label(label_exit));
            (temp, instructions)
        }
        Expr::And(left, right) => {
            let (l_addr, l_instructions) = translate_expression(*left, env);
            let (r_addr, r_instructions) = translate_expression(*right, env);
            let label_false = env.new_label();
            let label_exit = env.new_label();
            let temp = Address::Temporary(env.new_temporary(), Type::Bool);
            let mut instructions = l_instructions;
            instructions.push(Instruction::ConditionalJMPFalse(
                l_addr,
                label_false.clone(),
            ));
            instructions.extend(r_instructions);
            instructions.push(Instruction::ConditionalJMPFalse(
                r_addr,
                label_false.clone(),
            ));
            instructions.push(Instruction::CopyAssignment(
                temp.clone(),
                Address::Constant(Literal::Bool(true), Type::Bool),
            ));
            instructions.push(Instruction::JMP(label_exit.clone()));
            instructions.push(Instruction::Label(label_false));
            instructions.push(Instruction::CopyAssignment(
                temp.clone(),
                Address::Constant(Literal::Bool(false), Type::Bool),
            ));
            instructions.push(Instruction::Label(label_exit));
            (temp, instructions)
        }
        // Arithmetic Expressions
        Expr::Add(left, right) => {
            let (l_addr, l_instructions) = translate_expression(*left, env);
            let (r_addr, r_instructions) = translate_expression(*right, env);
            let mut instructions = [l_instructions, r_instructions].concat();
            let temp = Address::Temporary(env.new_temporary(), expression.ty);
            instructions.push(Instruction::BinaryAssignment(
                Operator::Add,
                temp.clone(),
                l_addr,
                r_addr,
            ));
            (temp, instructions)
        }
        Expr::Init { fields } => {
            let temp_name = env.new_struct_temp();
            let mut instructions = Vec::new();
            for (field_name, field_expr_opt) in fields {
                let field_expr = field_expr_opt.expect("struct field must have a value");
                let field_ty = field_expr.ty.clone();
                let (addr, insts) = translate_expression(field_expr, env);
                instructions.extend(insts);
                instructions.push(Instruction::CopyAssignment(
                    Address::Variable(format!("{}.{}", temp_name, field_name), field_ty),
                    addr,
                ));
            }
            (Address::Variable(temp_name, expression.ty), instructions)
        }
        Expr::EnumVariant {
            enum_name,
            variant,
            payload,
        } => {
            let temp_name = env.new_struct_temp();
            let enum_name = enum_name.unwrap_or_default();
            let ordinal = env.variant_tag_index(&enum_name, &variant);
            let mut instructions = vec![Instruction::CopyAssignment(
                Address::Variable(format!("{}.tag", temp_name), Type::Int),
                Address::Constant(Literal::Int(ordinal), Type::Int),
            )];
            if let Some(payload_expr) = payload {
                let payload_ty = payload_expr.ty.clone();
                let (addr, insts) = translate_expression(*payload_expr, env);
                instructions.extend(insts);
                instructions.push(Instruction::CopyAssignment(
                    Address::Variable(format!("{}.payload", temp_name), payload_ty),
                    addr,
                ));
            }
            (Address::Variable(temp_name, expression.ty), instructions)
        }
        Expr::Cast { ty: _cast_ty, expr } => {
            let (addr, insts) = translate_expression(*expr, env);
            let cast_addr = match addr {
                Address::Constant(lit, _) => Address::Constant(lit, expression.ty),
                Address::Variable(name, _) => Address::Variable(name, expression.ty),
                Address::Temporary(name, _) => Address::Temporary(name, expression.ty),
            };
            (cast_addr, insts)
        }
        _ => todo!(),
    }
}

fn translate_relational_false(
    left: CheckedExpr,
    right: CheckedExpr,
    op: Operator,
    false_label: String,
    env: &mut Environment,
) -> Vec<Instruction> {
    let (l_addr, l_instructions) = translate_expression(left, env);
    let (r_addr, r_instructions) = translate_expression(right, env);
    let mut instructions = l_instructions;
    instructions.extend(r_instructions);
    instructions.push(Instruction::ConditionalJMPRelational(
        op,
        l_addr,
        r_addr,
        false_label,
    ));
    instructions
}
