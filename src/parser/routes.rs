use std::borrow::Cow;
use std::path::Path;

use bumpalo::Bump;
use mago_database::file::File;
use mago_syntax::ast::{ArrayElement, Expression, Literal, Statement};
use mago_syntax::parser::parse_file;

use crate::model::RouteDefinition;
use crate::parser::ParseError;

use super::util::extract_string_value;

/// # Errors
/// Returns `ParseError` if the route file exists but cannot be parsed.
pub fn parse_routes(cache_dir: &Path) -> Result<Vec<RouteDefinition>, ParseError> {
    let route_file = cache_dir.join("url_matching_routes.php");
    if !route_file.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&route_file).map_err(|e| ParseError::Io {
        path: route_file.display().to_string(),
        source: e,
    })?;

    let arena = Bump::new();
    let file = File::ephemeral(
        Cow::Borrowed("url_matching_routes.php"),
        Cow::Owned(content),
    );
    let program = parse_file(&arena, &file);

    if program.has_errors() {
        return Err(ParseError::Php {
            file: "url_matching_routes.php".into(),
            message: "syntax errors in route file".into(),
        });
    }

    let root_elements = find_return_array(&program.statements)?;
    let mut routes = Vec::new();

    if let Some(static_routes) = get_array_element_by_index(root_elements, 1) {
        extract_routes_from_map(static_routes, &mut routes, true);
    }

    if let Some(dynamic_routes) = get_array_element_by_index(root_elements, 3) {
        extract_routes_from_map(dynamic_routes, &mut routes, false);
    }

    Ok(routes)
}

fn find_return_array<'a>(
    statements: &'a mago_syntax::ast::Sequence<'a, Statement<'a>>,
) -> Result<&'a [ArrayElement<'a>], ParseError> {
    for stmt in statements {
        if let Statement::Return(ret) = stmt
            && let Some(expr) = ret.value
        {
            return match expr {
                Expression::Array(arr) => Ok(arr.elements.as_slice()),
                Expression::LegacyArray(arr) => Ok(arr.elements.as_slice()),
                _ => Err(ParseError::Structure {
                    file: "url_matching_routes.php".into(),
                    detail: "return value is not an array".into(),
                }),
            };
        }
    }

    Err(ParseError::Structure {
        file: "url_matching_routes.php".into(),
        detail: "no return statement found".into(),
    })
}

fn get_array_element_by_index<'a>(
    elements: &'a [ArrayElement<'a>],
    index: usize,
) -> Option<&'a [ArrayElement<'a>]> {
    let mut positional = 0usize;
    for element in elements {
        match element {
            ArrayElement::KeyValue(kv) => {
                if let Expression::Literal(Literal::Integer(lit)) = kv.key
                    && lit.value == Some(index as u64)
                {
                    return array_elements_from_expr(kv.value);
                }
            }
            ArrayElement::Value(v) => {
                if positional == index {
                    return array_elements_from_expr(v.value);
                }
                positional += 1;
            }
            _ => {}
        }
    }
    None
}

fn array_elements_from_expr<'a>(expr: &'a Expression<'a>) -> Option<&'a [ArrayElement<'a>]> {
    match expr {
        Expression::Array(arr) => Some(arr.elements.as_slice()),
        Expression::LegacyArray(arr) => Some(arr.elements.as_slice()),
        _ => None,
    }
}

fn extract_routes_from_map(
    elements: &[ArrayElement<'_>],
    routes: &mut Vec<RouteDefinition>,
    is_static: bool,
) {
    for element in elements {
        let ArrayElement::KeyValue(kv) = element else {
            continue;
        };

        let path = if is_static {
            extract_string_value(kv.key).unwrap_or_default()
        } else {
            String::new()
        };

        let Some(route_entries) = array_elements_from_expr(kv.value) else {
            continue;
        };

        for route_entry in route_entries {
            let entry_elements = match route_entry {
                ArrayElement::Value(v) => array_elements_from_expr(v.value),
                ArrayElement::KeyValue(inner_kv) => array_elements_from_expr(inner_kv.value),
                _ => None,
            };
            let Some(entry_elements) = entry_elements else {
                continue;
            };

            let Some(attrs) = get_array_element_by_index(entry_elements, 0) else {
                continue;
            };

            let route_name = find_string_attr(attrs, "_route");
            let controller = find_string_attr(attrs, "_controller");

            let Some(name) = route_name else { continue };
            let Some(ctrl) = controller else { continue };

            let methods = get_array_element_by_index(entry_elements, 2)
                .map(extract_array_keys)
                .unwrap_or_default();

            routes.push(RouteDefinition {
                name,
                path: path.clone(),
                controller: ctrl,
                methods,
            });
        }
    }
}

fn find_string_attr(elements: &[ArrayElement<'_>], key: &str) -> Option<String> {
    for element in elements {
        if let ArrayElement::KeyValue(kv) = element
            && extract_string_value(kv.key).as_deref() == Some(key)
        {
            return extract_string_value(kv.value);
        }
    }
    None
}

fn extract_array_keys(elements: &[ArrayElement<'_>]) -> Vec<String> {
    let mut keys = Vec::new();
    for element in elements {
        if let ArrayElement::KeyValue(kv) = element
            && let Some(k) = extract_string_value(kv.key)
        {
            keys.push(k);
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_static_route() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("url_matching_routes.php"),
            r#"<?php return [false, ['/login' => [[['_route' => 'login', '_controller' => 'App\\Controller\\SecurityController::login'], null, null, null, false, false, null]]], [], [], null];"#,
        ).unwrap();
        let routes = parse_routes(dir.path()).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].name, "login");
        assert_eq!(
            routes[0].controller,
            "App\\Controller\\SecurityController::login"
        );
    }

    #[test]
    fn parse_dynamic_routes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("url_matching_routes.php"),
            r#"<?php return [false, [], [], [42 => [[['_route' => 'blog_index', '_controller' => 'App\\Controller\\BlogController::index'], ['_locale'], ['GET' => 0], null, false, false, null]], 99 => [[['_route' => 'blog_show', '_controller' => 'App\\Controller\\BlogController::show'], ['slug'], ['GET' => 0], null, false, false, null]]], null];"#,
        ).unwrap();
        let routes = parse_routes(dir.path()).unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].name, "blog_index");
        assert_eq!(routes[1].name, "blog_show");
    }

    #[test]
    fn missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let routes = parse_routes(dir.path()).unwrap();
        assert!(routes.is_empty());
    }

    #[test]
    fn route_with_http_methods() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("url_matching_routes.php"),
            r#"<?php return [false, [], [], [1 => [[['_route' => 'api_create', '_controller' => 'App\\Controller\\ApiController::create'], [], ['POST' => 0, 'PUT' => 1], null, false, false, null]]], null];"#,
        ).unwrap();
        let routes = parse_routes(dir.path()).unwrap();
        assert_eq!(routes[0].methods.len(), 2);
        assert!(routes[0].methods.contains(&"POST".to_owned()));
        assert!(routes[0].methods.contains(&"PUT".to_owned()));
    }
}
