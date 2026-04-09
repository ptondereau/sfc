use std::borrow::Cow;
use std::collections::HashSet;
use std::path::Path;

use bumpalo::Bump;
use mago_database::file::File;
use mago_syntax::ast::{
    Access, Argument, ArrayElement, AssignmentOperator, Block, ClassLikeMember,
    ClassLikeMemberSelector, Expression, Identifier, Instantiation, Literal, MethodBody, Statement,
    Variable,
};
use mago_syntax::parser::parse_file;

use crate::model::{Container, EdgeKind, Service, ServiceId, Visibility};
use crate::parser::ParseError;

use super::util::{extract_string_value, find_do_method_body, find_method_body_by_name};

/// # Errors
/// Returns `ParseError` if the main container file cannot be found or parsed.
pub fn parse_main_container(
    container_dir: &Path,
    container: &mut Container,
) -> Result<(), ParseError> {
    let main_file = find_main_container_file(container_dir)?;
    let content = std::fs::read_to_string(&main_file).map_err(|e| ParseError::Io {
        path: main_file.display().to_string(),
        source: e,
    })?;
    let file_name = main_file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let arena = Bump::new();
    let file = File::ephemeral(Cow::Owned(file_name.clone()), Cow::Owned(content));
    let program = parse_file(&arena, &file);

    if program.has_errors() {
        return Err(ParseError::Php {
            file: file_name,
            message: "syntax errors in main container file".into(),
        });
    }

    let constructor_body = find_constructor_body(&program.statements, &file_name)?;
    extract_maps_from_constructor(constructor_body, container);
    Ok(())
}

/// # Errors
/// Returns `ParseError` if the container directory cannot be read.
pub fn parse_service_factories(
    container_dir: &Path,
    container: &mut Container,
) -> Result<(), ParseError> {
    let entries = std::fs::read_dir(container_dir).map_err(|e| ParseError::Io {
        path: container_dir.display().to_string(),
        source: e,
    })?;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !name.starts_with("get") || !name.ends_with("Service.php") {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        let arena = Bump::new();
        let file = File::ephemeral(Cow::Owned(name.into_owned()), Cow::Owned(content));
        let program = parse_file(&arena, &file);

        if program.has_errors() {
            continue;
        }

        if let Some(block) = find_do_method_body(&program.statements) {
            extract_factory_info(block, &path, container);
        }
    }

    Ok(())
}

fn find_main_container_file(container_dir: &Path) -> Result<std::path::PathBuf, ParseError> {
    let entries = std::fs::read_dir(container_dir).map_err(|e| ParseError::Io {
        path: container_dir.display().to_string(),
        source: e,
    })?;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.contains("KernelProdContainer") || name_str.contains("KernelDevDebugContainer")
        {
            let meta = std::fs::metadata(&path).map_err(|e| ParseError::Io {
                path: path.display().to_string(),
                source: e,
            })?;
            if meta.len() > 5000 {
                return Ok(path);
            }
        }
    }

    Err(ParseError::Structure {
        file: container_dir.display().to_string(),
        detail: "no main container PHP file found (looking for *KernelProdContainer*.php > 5KB)"
            .into(),
    })
}

fn find_constructor_body<'a>(
    statements: &'a mago_syntax::ast::Sequence<'a, Statement<'a>>,
    file_name: &str,
) -> Result<&'a Block<'a>, ParseError> {
    for stmt in statements {
        if let Statement::Namespace(ns) = stmt {
            for inner in ns.statements() {
                if let Statement::Class(class) = inner {
                    return find_method_body_by_name(&class.members, "__construct").ok_or_else(
                        || ParseError::Structure {
                            file: file_name.to_owned(),
                            detail: "class found but no __construct method".into(),
                        },
                    );
                }
            }
        }
        if let Statement::Class(class) = stmt {
            return find_method_body_by_name(&class.members, "__construct").ok_or_else(|| {
                ParseError::Structure {
                    file: file_name.to_owned(),
                    detail: "class found but no __construct method".into(),
                }
            });
        }
    }

    Err(ParseError::Structure {
        file: file_name.to_owned(),
        detail: "no class found in container file".into(),
    })
}

fn extract_maps_from_constructor(block: &Block<'_>, container: &mut Container) {
    for stmt in &block.statements {
        let Statement::Expression(expr_stmt) = stmt else {
            continue;
        };

        let Expression::Assignment(assign) = expr_stmt.expression else {
            continue;
        };

        let Some(property_name) = extract_this_property_name(assign.lhs) else {
            continue;
        };

        match property_name {
            "methodMap" => extract_service_map(assign.rhs, Visibility::Public, container),
            "fileMap" => extract_service_map(assign.rhs, Visibility::Private, container),
            "aliases" => extract_alias_map(assign.rhs, container),
            _ => {}
        }
    }
}

fn extract_this_property_name<'a>(expr: &'a Expression<'_>) -> Option<&'a str> {
    if let Expression::Access(Access::Property(prop)) = expr
        && is_this_variable(prop.object)
        && let ClassLikeMemberSelector::Identifier(ident) = &prop.property
    {
        return Some(ident.value);
    }
    None
}

fn is_this_variable(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::Variable(Variable::Direct(dv)) if dv.name == "$this")
}

fn is_container_variable(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::Variable(Variable::Direct(dv)) if dv.name == "$container" || dv.name == "$this")
}

fn extract_service_map(expr: &Expression<'_>, visibility: Visibility, container: &mut Container) {
    let elements = match expr {
        Expression::Array(arr) => &arr.elements,
        Expression::LegacyArray(arr) => &arr.elements,
        _ => return,
    };

    for element in elements {
        if let ArrayElement::KeyValue(kv) = element
            && let Some(service_id) = extract_string_value(kv.key)
        {
            let id = ServiceId::new(service_id);
            if !container.services.contains_key(&id) {
                let _ = container.add_service(Service {
                    id,
                    class: String::new(),
                    factory_file: None,
                    tags: vec![],
                    visibility,
                    lazy: false,
                    roles: vec![],
                });
            }
        }
    }
}

fn extract_alias_map(expr: &Expression<'_>, container: &mut Container) {
    let elements = match expr {
        Expression::Array(arr) => &arr.elements,
        Expression::LegacyArray(arr) => &arr.elements,
        _ => return,
    };

    for element in elements {
        if let ArrayElement::KeyValue(kv) = element
            && let (Some(alias), Some(target)) =
                (extract_string_value(kv.key), extract_string_value(kv.value))
        {
            container
                .aliases
                .insert(ServiceId::new(alias), ServiceId::new(target));
        }
    }
}

fn extract_factory_info(block: &Block<'_>, factory_path: &Path, container: &mut Container) {
    let mut service_id: Option<ServiceId> = None;
    let mut class_name: Option<String> = None;
    let mut deps: Vec<ServiceId> = Vec::new();

    for stmt in &block.statements {
        match stmt {
            Statement::Return(ret) => {
                if let Some(expr) = ret.value {
                    scan_factory_expression(expr, &mut service_id, &mut class_name, &mut deps);
                }
            }
            Statement::Expression(expr_stmt) => {
                scan_factory_expression(
                    expr_stmt.expression,
                    &mut service_id,
                    &mut class_name,
                    &mut deps,
                );
            }
            _ => {}
        }
    }

    if let Some(sid) = service_id {
        if let Some(&idx) = container.services.get(&sid) {
            let svc = &mut container.graph[idx];
            if let Some(class) = &class_name {
                svc.class.clone_from(class);
            }
            svc.factory_file = Some(factory_path.to_path_buf());
        } else {
            let _ = container.add_service(Service {
                id: sid.clone(),
                class: class_name.unwrap_or_default(),
                factory_file: Some(factory_path.to_path_buf()),
                tags: vec![],
                visibility: Visibility::Private,
                lazy: false,
                roles: vec![],
            });
        }

        for dep_id in &deps {
            if !container.services.contains_key(dep_id) {
                let _ = container.add_service(Service {
                    id: dep_id.clone(),
                    class: String::new(),
                    factory_file: None,
                    tags: vec![],
                    visibility: Visibility::Private,
                    lazy: false,
                    roles: vec![],
                });
            }
            container.add_dependency(&sid, dep_id, EdgeKind::Constructor);
        }
    }
}

fn scan_factory_expression(
    expr: &Expression<'_>,
    service_id: &mut Option<ServiceId>,
    class_name: &mut Option<String>,
    deps: &mut Vec<ServiceId>,
) {
    match expr {
        Expression::Assignment(assign) => {
            if let Some(id) = extract_container_storage_key(assign.lhs) {
                *service_id = Some(ServiceId::new(id));
            }
            if matches!(assign.operator, AssignmentOperator::Coalesce(_)) {
                if let Expression::Instantiation(inst) = assign.rhs
                    && let Some(cn) = extract_class_name_from_instantiation(inst)
                {
                    deps.push(ServiceId::new(cn));
                }
                return;
            }
            scan_factory_expression(assign.rhs, service_id, class_name, deps);
        }
        Expression::Instantiation(inst) => {
            let class_fqcn = extract_class_name_from_instantiation(inst);
            if let Some(ref cn) = class_fqcn {
                *class_name = Some(cn.clone());
            }
            let is_service_locator = class_fqcn.is_some_and(|n| n.ends_with("ServiceLocator"));
            if is_service_locator {
                if let Some(ref arg_list) = inst.argument_list
                    && let Some(second_arg) = arg_list.arguments.get(1)
                {
                    let value = match second_arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    extract_array_keys_as_deps(value, deps);
                }
            } else if let Some(ref arg_list) = inst.argument_list {
                for arg in &arg_list.arguments {
                    let value = match arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    collect_dependency_refs(value, deps);
                }
            }
        }
        Expression::Parenthesized(p) => {
            scan_factory_expression(p.expression, service_id, class_name, deps);
        }
        Expression::Call(call) => {
            let arg_list = call.get_argument_list();
            for arg in &arg_list.arguments {
                let value = match arg {
                    Argument::Positional(p) => p.value,
                    Argument::Named(n) => n.value,
                };
                collect_dependency_refs(value, deps);
            }
        }
        _ => {}
    }
}

fn collect_dependency_refs(expr: &Expression<'_>, deps: &mut Vec<ServiceId>) {
    match expr {
        Expression::Binary(binop) => {
            if let Some(id) = extract_container_storage_key(binop.lhs) {
                deps.push(ServiceId::new(id));
            } else {
                collect_dependency_refs(binop.lhs, deps);
            }
            collect_dependency_refs(binop.rhs, deps);
        }
        Expression::Assignment(assign) => {
            if matches!(assign.operator, AssignmentOperator::Coalesce(_))
                && let Some(id) = extract_container_storage_key(assign.lhs)
            {
                deps.push(ServiceId::new(id));
                return;
            }
            collect_dependency_refs(assign.lhs, deps);
            collect_dependency_refs(assign.rhs, deps);
        }
        Expression::Parenthesized(p) => {
            collect_dependency_refs(p.expression, deps);
        }
        Expression::Call(call) => {
            let arg_list = call.get_argument_list();
            for arg in &arg_list.arguments {
                let value = match arg {
                    Argument::Positional(p) => p.value,
                    Argument::Named(n) => n.value,
                };
                collect_dependency_refs(value, deps);
            }
        }
        Expression::ArrayAccess(aa) => {
            if let Some(id) = extract_container_storage_key(expr) {
                deps.push(ServiceId::new(id));
            } else {
                collect_dependency_refs(aa.array, deps);
            }
        }
        Expression::Instantiation(inst) => {
            let is_service_locator = extract_class_name_from_instantiation(inst)
                .is_some_and(|name| name.ends_with("ServiceLocator"));
            if is_service_locator {
                if let Some(ref arg_list) = inst.argument_list
                    && let Some(second_arg) = arg_list.arguments.get(1)
                {
                    let value = match second_arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    extract_array_keys_as_deps(value, deps);
                }
            } else if let Some(ref arg_list) = inst.argument_list {
                for arg in &arg_list.arguments {
                    let value = match arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    collect_dependency_refs(value, deps);
                }
            }
        }
        Expression::Closure(closure) => {
            for stmt in &closure.body.statements {
                match stmt {
                    Statement::Return(ret) => {
                        if let Some(val) = ret.value {
                            collect_dependency_refs(val, deps);
                        }
                    }
                    Statement::Expression(expr_stmt) => {
                        collect_dependency_refs(expr_stmt.expression, deps);
                    }
                    _ => {}
                }
            }
        }
        Expression::ArrowFunction(arrow) => {
            collect_dependency_refs(arrow.expression, deps);
        }
        _ => {}
    }
}

fn extract_container_storage_key(expr: &Expression<'_>) -> Option<String> {
    if let Expression::ArrayAccess(aa) = expr
        && let Expression::Access(Access::Property(prop)) = aa.array
        && is_container_variable(prop.object)
        && let ClassLikeMemberSelector::Identifier(ident) = &prop.property
        && (ident.value == "services" || ident.value == "privates")
    {
        return extract_string_value(aa.index);
    }
    None
}

fn extract_class_name_from_instantiation(inst: &Instantiation<'_>) -> Option<String> {
    match inst.class {
        Expression::Identifier(Identifier::FullyQualified(fq)) => {
            Some(fq.value.trim_start_matches('\\').to_owned())
        }
        Expression::Identifier(Identifier::Qualified(q)) => Some(q.value.to_owned()),
        Expression::Identifier(Identifier::Local(l)) => Some(l.value.to_owned()),
        _ => None,
    }
}

fn extract_array_keys_as_deps(expr: &Expression<'_>, deps: &mut Vec<ServiceId>) {
    let elements = match expr {
        Expression::Array(arr) => &arr.elements,
        Expression::LegacyArray(arr) => &arr.elements,
        _ => return,
    };

    for element in elements {
        if let ArrayElement::KeyValue(kv) = element
            && let Some(key) = extract_string_value(kv.key)
        {
            deps.push(ServiceId::new(key));
        }
    }
}

/// # Errors
/// Returns `ParseError::Io` if the container directory cannot be read.
pub fn resolve_string_references(
    container_dir: &Path,
    container: &mut Container,
) -> Result<(), ParseError> {
    let known_ids: HashSet<String> = container.services.keys().map(|id| id.0.clone()).collect();

    let entries = std::fs::read_dir(container_dir).map_err(|e| ParseError::Io {
        path: container_dir.display().to_string(),
        source: e,
    })?;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("php") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };

        let arena = Bump::new();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let file = File::ephemeral(Cow::Owned(name.into_owned()), Cow::Owned(content));
        let program = parse_file(&arena, &file);
        if program.has_errors() {
            continue;
        }

        let mut refs = Vec::new();
        collect_all_string_literals(program.statements.as_slice(), &known_ids, &mut refs);

        match detect_factory_service_id(program.statements.as_slice()) {
            Some(from_id) => {
                for ref_id in refs {
                    if ref_id != from_id {
                        container.add_dependency(
                            &ServiceId::new(&from_id),
                            &ServiceId::new(ref_id),
                            EdgeKind::Constructor,
                        );
                    }
                }
            }
            None => {
                for ref_id in refs {
                    container.kernel_referenced.insert(ServiceId::new(ref_id));
                }
            }
        }
    }

    Ok(())
}

fn detect_factory_service_id(statements: &[Statement<'_>]) -> Option<String> {
    for stmt in statements {
        match stmt {
            Statement::Namespace(ns) => {
                let result = detect_factory_service_id(ns.statements().as_slice());
                if result.is_some() {
                    return result;
                }
            }
            Statement::Class(class) => {
                if let Some(block) = find_method_body_by_name(&class.members, "do") {
                    return scan_block_for_service_assignment(block);
                }
            }
            _ => {}
        }
    }
    None
}

fn scan_block_for_service_assignment(block: &Block<'_>) -> Option<String> {
    for stmt in &block.statements {
        match stmt {
            Statement::Expression(expr_stmt) => {
                if let Some(id) = scan_expr_for_service_assignment(expr_stmt.expression) {
                    return Some(id);
                }
            }
            Statement::Return(ret) => {
                if let Some(expr) = ret.value
                    && let Some(id) = scan_expr_for_service_assignment(expr)
                {
                    return Some(id);
                }
            }
            _ => {}
        }
    }
    None
}

fn scan_expr_for_service_assignment(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::Assignment(assign) => {
            if let Some(id) = extract_container_storage_key(assign.lhs) {
                return Some(id);
            }
            scan_expr_for_service_assignment(assign.rhs)
        }
        Expression::Parenthesized(p) => scan_expr_for_service_assignment(p.expression),
        _ => None,
    }
}

fn collect_all_string_literals(
    statements: &[Statement<'_>],
    known_ids: &HashSet<String>,
    refs: &mut Vec<String>,
) {
    for stmt in statements {
        walk_statement(stmt, known_ids, refs);
    }
}

#[allow(clippy::too_many_lines)]
fn walk_statement(stmt: &Statement<'_>, known_ids: &HashSet<String>, refs: &mut Vec<String>) {
    match stmt {
        Statement::Namespace(ns) => {
            collect_all_string_literals(ns.statements().as_slice(), known_ids, refs);
        }
        Statement::Class(class) => walk_class_members(&class.members, known_ids, refs),
        Statement::Interface(iface) => walk_class_members(&iface.members, known_ids, refs),
        Statement::Trait(tr) => walk_class_members(&tr.members, known_ids, refs),
        Statement::Enum(en) => walk_class_members(&en.members, known_ids, refs),
        Statement::Block(block) => {
            collect_all_string_literals(block.statements.as_slice(), known_ids, refs);
        }
        Statement::Function(func) => {
            collect_all_string_literals(func.body.statements.as_slice(), known_ids, refs);
        }
        Statement::Return(ret) => {
            if let Some(val) = ret.value {
                walk_expression(val, known_ids, refs);
            }
        }
        Statement::Expression(expr_stmt) => {
            walk_expression(expr_stmt.expression, known_ids, refs);
        }
        Statement::If(if_stmt) => {
            walk_expression(if_stmt.condition, known_ids, refs);
            for s in if_stmt.body.statements() {
                walk_statement(s, known_ids, refs);
            }
            for (cond, stmts) in if_stmt.body.else_if_clauses() {
                walk_expression(cond, known_ids, refs);
                for s in stmts {
                    walk_statement(s, known_ids, refs);
                }
            }
            if let Some(else_stmts) = if_stmt.body.else_statements() {
                for s in else_stmts {
                    walk_statement(s, known_ids, refs);
                }
            }
        }
        Statement::Switch(switch) => {
            walk_expression(switch.expression, known_ids, refs);
            for case in switch.body.cases() {
                if let Some(expr) = case.expression() {
                    walk_expression(expr, known_ids, refs);
                }
                for s in case.statements() {
                    walk_statement(s, known_ids, refs);
                }
            }
        }
        Statement::Try(try_stmt) => {
            collect_all_string_literals(try_stmt.block.statements.as_slice(), known_ids, refs);
            for catch in &try_stmt.catch_clauses {
                collect_all_string_literals(catch.block.statements.as_slice(), known_ids, refs);
            }
            if let Some(finally) = &try_stmt.finally_clause {
                collect_all_string_literals(finally.block.statements.as_slice(), known_ids, refs);
            }
        }
        Statement::Foreach(foreach) => {
            walk_expression(foreach.expression, known_ids, refs);
            for s in foreach.body.statements() {
                walk_statement(s, known_ids, refs);
            }
        }
        Statement::For(for_stmt) => {
            for expr in &for_stmt.initializations {
                walk_expression(expr, known_ids, refs);
            }
            for expr in &for_stmt.conditions {
                walk_expression(expr, known_ids, refs);
            }
            for expr in &for_stmt.increments {
                walk_expression(expr, known_ids, refs);
            }
            for s in for_stmt.body.statements() {
                walk_statement(s, known_ids, refs);
            }
        }
        Statement::While(while_stmt) => {
            walk_expression(while_stmt.condition, known_ids, refs);
            for s in while_stmt.body.statements() {
                walk_statement(s, known_ids, refs);
            }
        }
        Statement::DoWhile(dw) => {
            walk_statement(dw.statement, known_ids, refs);
            walk_expression(dw.condition, known_ids, refs);
        }
        Statement::Echo(echo) => {
            for expr in &echo.values {
                walk_expression(expr, known_ids, refs);
            }
        }
        Statement::Unset(unset) => {
            for expr in &unset.values {
                walk_expression(expr, known_ids, refs);
            }
        }
        Statement::Declare(decl) => match &decl.body {
            mago_syntax::ast::DeclareBody::Statement(s) => walk_statement(s, known_ids, refs),
            mago_syntax::ast::DeclareBody::ColonDelimited(c) => {
                collect_all_string_literals(c.statements.as_slice(), known_ids, refs);
            }
        },
        Statement::Global(global) => {
            for var in &global.variables {
                if let Variable::Indirect(iv) = var {
                    walk_expression(iv.expression, known_ids, refs);
                }
            }
        }
        Statement::Static(static_stmt) => {
            for item in &static_stmt.items {
                if let Some(val) = item.value() {
                    walk_expression(val, known_ids, refs);
                }
            }
        }
        _ => {}
    }
}

fn walk_class_members(
    members: &mago_syntax::ast::Sequence<'_, ClassLikeMember<'_>>,
    known_ids: &HashSet<String>,
    refs: &mut Vec<String>,
) {
    for member in members {
        match member {
            ClassLikeMember::Method(method) => {
                if let MethodBody::Concrete(block) = &method.body {
                    collect_all_string_literals(block.statements.as_slice(), known_ids, refs);
                }
            }
            ClassLikeMember::Property(prop) => {
                walk_property(prop, known_ids, refs);
            }
            ClassLikeMember::Constant(constant) => {
                for item in &constant.items {
                    walk_expression(item.value, known_ids, refs);
                }
            }
            ClassLikeMember::EnumCase(ec) => {
                if let mago_syntax::ast::EnumCaseItem::Backed(backed) = &ec.item {
                    walk_expression(backed.value, known_ids, refs);
                }
            }
            ClassLikeMember::TraitUse(_) => {}
        }
    }
}

fn walk_property(
    prop: &mago_syntax::ast::Property<'_>,
    known_ids: &HashSet<String>,
    refs: &mut Vec<String>,
) {
    match prop {
        mago_syntax::ast::Property::Plain(plain) => {
            for item in &plain.items {
                if let mago_syntax::ast::PropertyItem::Concrete(c) = item {
                    walk_expression(c.value, known_ids, refs);
                }
            }
        }
        mago_syntax::ast::Property::Hooked(hooked) => {
            if let mago_syntax::ast::PropertyItem::Concrete(c) = &hooked.item {
                walk_expression(c.value, known_ids, refs);
            }
            for hook in &hooked.hook_list.hooks {
                match &hook.body {
                    mago_syntax::ast::PropertyHookBody::Concrete(
                        mago_syntax::ast::PropertyHookConcreteBody::Expression(expr),
                    ) => {
                        walk_expression(expr.expression, known_ids, refs);
                    }
                    mago_syntax::ast::PropertyHookBody::Concrete(
                        mago_syntax::ast::PropertyHookConcreteBody::Block(block),
                    ) => {
                        collect_all_string_literals(block.statements.as_slice(), known_ids, refs);
                    }
                    mago_syntax::ast::PropertyHookBody::Abstract(_) => {}
                }
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn walk_expression(expr: &Expression<'_>, known_ids: &HashSet<String>, refs: &mut Vec<String>) {
    match expr {
        Expression::Literal(Literal::String(s)) => {
            if let Some(val) = s.value
                && known_ids.contains(val)
            {
                refs.push(val.to_owned());
            }
        }
        Expression::Binary(bin) => {
            walk_expression(bin.lhs, known_ids, refs);
            walk_expression(bin.rhs, known_ids, refs);
        }
        Expression::UnaryPrefix(u) => {
            walk_expression(u.operand, known_ids, refs);
        }
        Expression::UnaryPostfix(u) => {
            walk_expression(u.operand, known_ids, refs);
        }
        Expression::Parenthesized(p) => {
            walk_expression(p.expression, known_ids, refs);
        }
        Expression::Assignment(assign) => {
            walk_expression(assign.lhs, known_ids, refs);
            walk_expression(assign.rhs, known_ids, refs);
        }
        Expression::Conditional(cond) => {
            walk_expression(cond.condition, known_ids, refs);
            if let Some(then) = cond.then {
                walk_expression(then, known_ids, refs);
            }
            walk_expression(cond.r#else, known_ids, refs);
        }
        Expression::Array(arr) => {
            walk_array_elements(arr.elements.as_slice(), known_ids, refs);
        }
        Expression::LegacyArray(arr) => {
            walk_array_elements(arr.elements.as_slice(), known_ids, refs);
        }
        Expression::List(list) => {
            walk_array_elements(list.elements.as_slice(), known_ids, refs);
        }
        Expression::ArrayAccess(aa) => {
            walk_expression(aa.array, known_ids, refs);
            walk_expression(aa.index, known_ids, refs);
        }
        Expression::ArrayAppend(aa) => {
            walk_expression(aa.array, known_ids, refs);
        }
        Expression::Call(call) => {
            let arg_list = call.get_argument_list();
            for arg in &arg_list.arguments {
                let val = match arg {
                    Argument::Positional(p) => p.value,
                    Argument::Named(n) => n.value,
                };
                walk_expression(val, known_ids, refs);
            }
            match call {
                mago_syntax::ast::Call::Function(f) => {
                    walk_expression(f.function, known_ids, refs);
                }
                mago_syntax::ast::Call::Method(m) => {
                    walk_expression(m.object, known_ids, refs);
                }
                mago_syntax::ast::Call::NullSafeMethod(n) => {
                    walk_expression(n.object, known_ids, refs);
                }
                mago_syntax::ast::Call::StaticMethod(s) => {
                    walk_expression(s.class, known_ids, refs);
                }
            }
        }
        Expression::Instantiation(inst) => {
            walk_expression(inst.class, known_ids, refs);
            if let Some(arg_list) = &inst.argument_list {
                for arg in &arg_list.arguments {
                    let val = match arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    walk_expression(val, known_ids, refs);
                }
            }
        }
        Expression::Closure(closure) => {
            collect_all_string_literals(closure.body.statements.as_slice(), known_ids, refs);
        }
        Expression::ArrowFunction(arrow) => {
            walk_expression(arrow.expression, known_ids, refs);
        }
        Expression::Access(access) => match access {
            Access::Property(p) => walk_expression(p.object, known_ids, refs),
            Access::NullSafeProperty(n) => walk_expression(n.object, known_ids, refs),
            Access::StaticProperty(s) => walk_expression(s.class, known_ids, refs),
            Access::ClassConstant(c) => walk_expression(c.class, known_ids, refs),
        },
        Expression::Match(m) => {
            walk_expression(m.expression, known_ids, refs);
            for arm in &m.arms {
                match arm {
                    mago_syntax::ast::MatchArm::Expression(e) => {
                        for cond in &e.conditions {
                            walk_expression(cond, known_ids, refs);
                        }
                        walk_expression(e.expression, known_ids, refs);
                    }
                    mago_syntax::ast::MatchArm::Default(d) => {
                        walk_expression(d.expression, known_ids, refs);
                    }
                }
            }
        }
        Expression::Yield(y) => match y {
            mago_syntax::ast::Yield::Value(v) => {
                if let Some(val) = v.value {
                    walk_expression(val, known_ids, refs);
                }
            }
            mago_syntax::ast::Yield::Pair(p) => {
                walk_expression(p.key, known_ids, refs);
                walk_expression(p.value, known_ids, refs);
            }
            mago_syntax::ast::Yield::From(f) => {
                walk_expression(f.iterator, known_ids, refs);
            }
        },
        Expression::Construct(construct) => {
            walk_construct(construct, known_ids, refs);
        }
        Expression::Throw(t) => {
            walk_expression(t.exception, known_ids, refs);
        }
        Expression::Clone(c) => {
            walk_expression(c.object, known_ids, refs);
        }
        Expression::Pipe(pipe) => {
            walk_expression(pipe.input, known_ids, refs);
            walk_expression(pipe.callable, known_ids, refs);
        }
        Expression::AnonymousClass(anon) => {
            if let Some(arg_list) = &anon.argument_list {
                for arg in &arg_list.arguments {
                    let val = match arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    walk_expression(val, known_ids, refs);
                }
            }
            walk_class_members(&anon.members, known_ids, refs);
        }
        Expression::CompositeString(cs) => {
            for part in cs.parts() {
                match part {
                    mago_syntax::ast::StringPart::Expression(e) => {
                        walk_expression(e, known_ids, refs);
                    }
                    mago_syntax::ast::StringPart::BracedExpression(b) => {
                        walk_expression(b.expression, known_ids, refs);
                    }
                    mago_syntax::ast::StringPart::Literal(_) => {}
                }
            }
        }
        _ => {}
    }
}

fn walk_array_elements(
    elements: &[ArrayElement<'_>],
    known_ids: &HashSet<String>,
    refs: &mut Vec<String>,
) {
    for element in elements {
        match element {
            ArrayElement::KeyValue(kv) => {
                walk_expression(kv.key, known_ids, refs);
                walk_expression(kv.value, known_ids, refs);
            }
            ArrayElement::Value(v) => {
                walk_expression(v.value, known_ids, refs);
            }
            ArrayElement::Variadic(v) => {
                walk_expression(v.value, known_ids, refs);
            }
            ArrayElement::Missing(_) => {}
        }
    }
}

fn walk_construct(
    construct: &mago_syntax::ast::Construct<'_>,
    known_ids: &HashSet<String>,
    refs: &mut Vec<String>,
) {
    match construct {
        mago_syntax::ast::Construct::Isset(c) => {
            for val in &c.values {
                walk_expression(val, known_ids, refs);
            }
        }
        mago_syntax::ast::Construct::Empty(c) => walk_expression(c.value, known_ids, refs),
        mago_syntax::ast::Construct::Eval(c) => walk_expression(c.value, known_ids, refs),
        mago_syntax::ast::Construct::Include(c) => walk_expression(c.value, known_ids, refs),
        mago_syntax::ast::Construct::IncludeOnce(c) => walk_expression(c.value, known_ids, refs),
        mago_syntax::ast::Construct::Require(c) => walk_expression(c.value, known_ids, refs),
        mago_syntax::ast::Construct::RequireOnce(c) => walk_expression(c.value, known_ids, refs),
        mago_syntax::ast::Construct::Print(c) => walk_expression(c.value, known_ids, refs),
        mago_syntax::ast::Construct::Exit(c) => {
            if let Some(args) = &c.arguments {
                for arg in &args.arguments {
                    let val = match arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    walk_expression(val, known_ids, refs);
                }
            }
        }
        mago_syntax::ast::Construct::Die(c) => {
            if let Some(args) = &c.arguments {
                for arg in &args.arguments {
                    let val = match arg {
                        Argument::Positional(p) => p.value,
                        Argument::Named(n) => n.value,
                    };
                    walk_expression(val, known_ids, refs);
                }
            }
        }
    }
}
