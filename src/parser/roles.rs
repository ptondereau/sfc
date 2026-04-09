use std::borrow::Cow;
use std::path::Path;

use bumpalo::Bump;
use mago_database::file::File;
use mago_syntax::ast::{
    Argument, ArrayElement, Call, ClassLikeMemberSelector, Expression, Identifier, Literal,
    Statement, UnaryPrefixOperator,
};
use mago_syntax::parser::parse_file;
use petgraph::graph::NodeIndex;

use crate::model::{Container, ServiceId, ServiceRole};
use crate::parser::ParseError;

/// # Errors
/// Returns `ParseError` if factory files cannot be read.
pub fn infer_roles(container_dir: &Path, container: &mut Container) -> Result<(), ParseError> {
    infer_event_listeners(container_dir, container)?;
    infer_console_commands(container_dir, container)?;
    infer_voters(container);
    Ok(())
}

fn infer_event_listeners(
    container_dir: &Path,
    container: &mut Container,
) -> Result<(), ParseError> {
    let factory_path = container_dir.join("getEventDispatcherService.php");
    if !factory_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&factory_path).map_err(|e| ParseError::Io {
        path: factory_path.display().to_string(),
        source: e,
    })?;

    let arena = Bump::new();
    let file = File::ephemeral(
        Cow::Borrowed("getEventDispatcherService.php"),
        Cow::Owned(content),
    );
    let program = parse_file(&arena, &file);
    if program.has_errors() {
        return Err(ParseError::Php {
            file: "getEventDispatcherService.php".into(),
            message: "syntax errors in event dispatcher factory".into(),
        });
    }

    let Some(block) = find_do_method_body(&program.statements) else {
        return Ok(());
    };

    let mut listeners: Vec<(String, String, String, i32)> = Vec::new();

    for stmt in &block.statements {
        let Statement::Expression(expr_stmt) = stmt else {
            continue;
        };
        let Expression::Call(Call::Method(method_call)) = expr_stmt.expression else {
            continue;
        };
        let ClassLikeMemberSelector::Identifier(ident) = &method_call.method else {
            continue;
        };
        if ident.value != "addListener" {
            continue;
        }

        let args = &method_call.argument_list.arguments;
        let Some(event_name) = args.get(0).and_then(|a| extract_string_from_arg(a)) else {
            continue;
        };

        let Some(array_arg) = args.get(1).map(arg_value) else {
            continue;
        };
        let (service_id, method) = extract_listener_callback(array_arg);
        let Some(service_id) = service_id else {
            continue;
        };
        let Some(method) = method else {
            continue;
        };

        let priority = args
            .get(2)
            .map(arg_value)
            .and_then(|e| extract_integer_value(e))
            .unwrap_or(0);

        listeners.push((service_id, event_name, method, priority));
    }

    for (service_id, event, method, priority) in listeners {
        let sid = ServiceId::new(&service_id);
        if let Some(&idx) = container.services.get(&sid) {
            container.graph[idx].roles.push(ServiceRole::EventListener {
                event,
                method,
                priority,
            });
        }
    }

    Ok(())
}

fn extract_listener_callback(expr: &Expression<'_>) -> (Option<String>, Option<String>) {
    let elements = match expr {
        Expression::Array(arr) => &arr.elements,
        Expression::LegacyArray(arr) => &arr.elements,
        _ => return (None, None),
    };

    let service_id = elements.get(0).and_then(|el| {
        let ArrayElement::Value(v) = el else {
            return None;
        };
        extract_service_id_from_closure(v.value)
    });

    let method = elements.get(1).and_then(|el| {
        let ArrayElement::Value(v) = el else {
            return None;
        };
        extract_string_value(v.value)
    });

    (service_id, method)
}

fn extract_service_id_from_closure(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::ArrowFunction(arrow) => extract_container_key(arrow.expression),
        Expression::Closure(closure) => {
            for stmt in &closure.body.statements {
                if let Statement::Return(ret) = stmt
                    && let Some(val) = ret.value
                    && let Some(id) = extract_container_key(val)
                {
                    return Some(id);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_container_key(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::Parenthesized(p) => extract_container_key(p.expression),
        Expression::Binary(bin) => {
            extract_container_key(bin.lhs).or_else(|| extract_container_key(bin.rhs))
        }
        Expression::ArrayAccess(aa) => {
            if let Expression::Access(mago_syntax::ast::Access::Property(prop)) = aa.array
                && let ClassLikeMemberSelector::Identifier(ident) = &prop.property
                && (ident.value == "privates" || ident.value == "services")
            {
                return extract_string_value(aa.index);
            }
            None
        }
        _ => None,
    }
}

fn infer_console_commands(
    container_dir: &Path,
    container: &mut Container,
) -> Result<(), ParseError> {
    let factory_path = container_dir.join("getConsole_CommandLoaderService.php");
    if !factory_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&factory_path).map_err(|e| ParseError::Io {
        path: factory_path.display().to_string(),
        source: e,
    })?;

    let arena = Bump::new();
    let file = File::ephemeral(
        Cow::Borrowed("getConsole_CommandLoaderService.php"),
        Cow::Owned(content),
    );
    let program = parse_file(&arena, &file);
    if program.has_errors() {
        return Err(ParseError::Php {
            file: "getConsole_CommandLoaderService.php".into(),
            message: "syntax errors in command loader factory".into(),
        });
    }

    let Some(block) = find_do_method_body(&program.statements) else {
        return Ok(());
    };

    let mut commands: Vec<(String, String)> = Vec::new();
    collect_command_mappings(block.statements.as_slice(), &mut commands);

    for (command_name, service_id) in commands {
        let sid = ServiceId::new(&service_id);
        if let Some(&idx) = container.services.get(&sid) {
            container.graph[idx]
                .roles
                .push(ServiceRole::ConsoleCommand { command_name });
        }
    }

    Ok(())
}

fn collect_command_mappings(statements: &[Statement<'_>], commands: &mut Vec<(String, String)>) {
    for stmt in statements {
        match stmt {
            Statement::Return(ret) => {
                if let Some(expr) = ret.value {
                    scan_expr_for_command_loader(expr, commands);
                }
            }
            Statement::Expression(expr_stmt) => {
                scan_expr_for_command_loader(expr_stmt.expression, commands);
            }
            _ => {}
        }
    }
}

fn scan_expr_for_command_loader(expr: &Expression<'_>, commands: &mut Vec<(String, String)>) {
    match expr {
        Expression::Assignment(assign) => {
            scan_expr_for_command_loader(assign.rhs, commands);
        }
        Expression::Parenthesized(p) => {
            scan_expr_for_command_loader(p.expression, commands);
        }
        Expression::Instantiation(inst) => {
            let class_name = match inst.class {
                Expression::Identifier(Identifier::FullyQualified(fq)) => {
                    Some(fq.value.trim_start_matches('\\'))
                }
                Expression::Identifier(Identifier::Qualified(q)) => Some(q.value),
                Expression::Identifier(Identifier::Local(l)) => Some(l.value),
                _ => None,
            };

            if class_name.is_some_and(|n| n.ends_with("CommandLoader"))
                && let Some(ref arg_list) = inst.argument_list
                && let Some(second_arg) = arg_list.arguments.get(1)
            {
                extract_command_map(arg_value(second_arg), commands);
            }
        }
        _ => {}
    }
}

fn extract_command_map(expr: &Expression<'_>, commands: &mut Vec<(String, String)>) {
    let elements = match expr {
        Expression::Array(arr) => &arr.elements,
        Expression::LegacyArray(arr) => &arr.elements,
        _ => return,
    };

    for element in elements {
        if let ArrayElement::KeyValue(kv) = element
            && let Some(command_name) = extract_string_value(kv.key)
            && let Some(service_id) = extract_string_value(kv.value)
        {
            commands.push((command_name, service_id));
        }
    }
}

fn infer_voters(container: &mut Container) {
    let voter_indices: Vec<NodeIndex> = container
        .services
        .values()
        .copied()
        .filter(|&idx| container.graph[idx].class.ends_with("Voter"))
        .collect();

    for idx in voter_indices {
        container.graph[idx].roles.push(ServiceRole::Voter);
    }
}

fn find_do_method_body<'a>(
    statements: &'a mago_syntax::ast::Sequence<'a, Statement<'a>>,
) -> Option<&'a mago_syntax::ast::Block<'a>> {
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

fn find_method_body_by_name<'a>(
    members: &'a mago_syntax::ast::Sequence<'a, mago_syntax::ast::ClassLikeMember<'a>>,
    name: &str,
) -> Option<&'a mago_syntax::ast::Block<'a>> {
    for member in members {
        if let mago_syntax::ast::ClassLikeMember::Method(method) = member
            && method.name.value == name
            && let mago_syntax::ast::MethodBody::Concrete(block) = &method.body
        {
            return Some(block);
        }
    }
    None
}

fn extract_string_value(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::Literal(Literal::String(s)) => s.value.map(ToOwned::to_owned),
        _ => None,
    }
}

fn extract_string_from_arg(arg: &Argument<'_>) -> Option<String> {
    extract_string_value(arg_value(arg))
}

fn extract_integer_value(expr: &Expression<'_>) -> Option<i32> {
    match expr {
        Expression::Literal(Literal::Integer(i)) => i.value.and_then(|v| i32::try_from(v).ok()),
        Expression::UnaryPrefix(u) if matches!(u.operator, UnaryPrefixOperator::Negation(_)) => {
            if let Expression::Literal(Literal::Integer(i)) = u.operand {
                i.value.and_then(|v| i32::try_from(v).ok()).map(|v| -v)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn arg_value<'a>(arg: &'a Argument<'a>) -> &'a Expression<'a> {
    match arg {
        Argument::Positional(p) => p.value,
        Argument::Named(n) => n.value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::fs;
    use std::path::PathBuf;

    fn setup_event_dispatcher_factory(dir: &Path) {
        fs::write(
            dir.join("getEventDispatcherService.php"),
            r#"<?php
namespace ContainerTest;

class getEventDispatcherService extends App_KernelProdContainer
{
    public static function do($container, $lazyLoad = true)
    {
        $container->services['event_dispatcher'] = $instance = new \Symfony\Component\EventDispatcher\EventDispatcher();
        $instance->addListener('kernel.request',
            [fn () => ($container->privates['my.listener'] ?? $container->load('getMyListenerService')), 'onRequest'],
            10
        );
        return $instance;
    }
}
"#,
        )
        .unwrap();
    }

    #[test]
    fn infer_event_listener_role() {
        let dir = tempfile::tempdir().unwrap();
        setup_event_dispatcher_factory(dir.path());

        let mut container = Container::new(PathBuf::from("/tmp"));
        let _ = container.add_service(Service {
            id: ServiceId::new("my.listener"),
            class: "MyListener".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });

        infer_event_listeners(dir.path(), &mut container).unwrap();

        let idx = container
            .services
            .get(&ServiceId::new("my.listener"))
            .unwrap();
        let service = &container.graph[*idx];
        assert!(!service.roles.is_empty());
        assert!(
            matches!(&service.roles[0], ServiceRole::EventListener { event, .. } if event == "kernel.request")
        );
    }

    #[test]
    fn infer_voter_by_class_name() {
        let mut container = Container::new(PathBuf::from("/tmp"));
        let _ = container.add_service(Service {
            id: ServiceId::new("app.post_voter"),
            class: "App\\Security\\PostVoter".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });

        infer_voters(&mut container);

        let idx = container
            .services
            .get(&ServiceId::new("app.post_voter"))
            .unwrap();
        assert!(matches!(
            &container.graph[*idx].roles[0],
            ServiceRole::Voter
        ));
    }

    #[test]
    fn infer_console_command_role() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("getConsole_CommandLoaderService.php"),
            r#"<?php
namespace ContainerTest;

class getConsole_CommandLoaderService extends App_KernelProdContainer
{
    public static function do($container, $lazyLoad = true)
    {
        return $container->services['console.command_loader'] = new \Symfony\Component\Console\CommandLoader\ContainerCommandLoader(new \Symfony\Component\DependencyInjection\Argument\ServiceLocator($container->getService(...), []), [
            'app:add-user' => 'App\\Command\\AddUserCommand',
            'about' => 'console.command.about',
        ]);
    }
}
"#,
        )
        .unwrap();

        let mut container = Container::new(PathBuf::from("/tmp"));
        let _ = container.add_service(Service {
            id: ServiceId::new("App\\Command\\AddUserCommand"),
            class: "App\\Command\\AddUserCommand".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });
        let _ = container.add_service(Service {
            id: ServiceId::new("console.command.about"),
            class: "Symfony\\Component\\Console\\Command\\AboutCommand".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });

        infer_console_commands(dir.path(), &mut container).unwrap();

        let idx = container
            .services
            .get(&ServiceId::new("App\\Command\\AddUserCommand"))
            .unwrap();
        assert!(
            matches!(&container.graph[*idx].roles[0], ServiceRole::ConsoleCommand { command_name } if command_name == "app:add-user")
        );

        let idx2 = container
            .services
            .get(&ServiceId::new("console.command.about"))
            .unwrap();
        assert!(
            matches!(&container.graph[*idx2].roles[0], ServiceRole::ConsoleCommand { command_name } if command_name == "about")
        );
    }

    #[test]
    fn missing_factory_files_are_not_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut container = Container::new(PathBuf::from("/tmp"));
        infer_event_listeners(dir.path(), &mut container).unwrap();
        infer_console_commands(dir.path(), &mut container).unwrap();
    }

    #[test]
    fn non_voter_class_not_tagged() {
        let mut container = Container::new(PathBuf::from("/tmp"));
        let _ = container.add_service(Service {
            id: ServiceId::new("app.mailer"),
            class: "App\\Service\\Mailer".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });

        infer_voters(&mut container);

        let idx = container
            .services
            .get(&ServiceId::new("app.mailer"))
            .unwrap();
        assert!(container.graph[*idx].roles.is_empty());
    }
}
