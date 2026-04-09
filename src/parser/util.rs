use mago_syntax::ast::{Block, ClassLikeMember, Expression, Literal, MethodBody, Statement};

#[must_use]
pub fn extract_string_value(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::Literal(Literal::String(s)) => s.value.map(ToOwned::to_owned),
        _ => None,
    }
}

#[must_use]
pub fn find_do_method_body<'a>(
    statements: &'a mago_syntax::ast::Sequence<'a, Statement<'a>>,
) -> Option<&'a Block<'a>> {
    for stmt in statements {
        if let Statement::Namespace(ns) = stmt {
            for inner in ns.statements() {
                if let Statement::Class(class) = inner {
                    return find_method_body_by_name(&class.members, "do");
                }
            }
        }
        if let Statement::Class(class) = stmt {
            return find_method_body_by_name(&class.members, "do");
        }
    }
    None
}

#[must_use]
pub fn find_method_body_by_name<'a>(
    members: &'a mago_syntax::ast::Sequence<'a, ClassLikeMember<'a>>,
    name: &str,
) -> Option<&'a Block<'a>> {
    for member in members {
        if let ClassLikeMember::Method(method) = member
            && method.name.value == name
            && let MethodBody::Concrete(block) = &method.body
        {
            return Some(block);
        }
    }
    None
}
