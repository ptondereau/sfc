use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bumpalo::Bump;
use mago_database::file::File;
use mago_syntax::ast::{ClassLikeMember, Identifier, MethodBody, Statement};
use mago_syntax::parser::parse_file;

const ZEND_OBJECT_BASE: u64 = 72;
const ZVAL_SLOT: u64 = 16;

/// Per-class introspection result.
#[derive(Debug, Clone)]
pub struct ClassInfo {
    #[allow(dead_code)]
    pub fqcn: String,
    pub property_count: u32,
    pub parent: Option<String>,
    #[allow(dead_code)]
    pub estimated_bytes: u64,
}

/// Resolves FQCN → file path using composer's classmap and PSR-4 dump.
pub struct ClassResolver {
    classmap: HashMap<String, PathBuf>,
    psr4: Vec<(String, PathBuf)>,
}

impl ClassResolver {
    #[must_use]
    pub fn from_project(project_root: &Path) -> Self {
        let vendor = project_root.join("vendor/composer");
        let classmap = parse_classmap(&vendor.join("autoload_classmap.php"), project_root);
        let psr4 = parse_psr4(&vendor.join("autoload_psr4.php"), project_root);

        Self { classmap, psr4 }
    }

    #[must_use]
    pub fn resolve(&self, fqcn: &str) -> Option<PathBuf> {
        if let Some(path) = self.classmap.get(fqcn)
            && path.exists()
        {
            return Some(path.clone());
        }

        for (prefix, dir) in &self.psr4 {
            if let Some(rest) = fqcn.strip_prefix(prefix.as_str()) {
                let relative = rest.replace('\\', "/") + ".php";
                let path = dir.join(relative);
                if path.exists() {
                    return Some(path);
                }
            }
        }

        None
    }
}

/// Introspects a PHP class file and counts declared properties.
///
/// Counts both explicit property declarations and promoted constructor parameters.
#[must_use]
pub fn introspect_class(path: &Path) -> Option<ClassInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    let file_name = path.file_name()?.to_string_lossy().into_owned();

    let arena = Bump::new();
    let file = File::ephemeral(Cow::Owned(file_name), Cow::Owned(content));
    let program = parse_file(&arena, &file);

    if program.has_errors() {
        return None;
    }

    extract_class_info(&program.statements)
}

fn extract_class_info(
    statements: &mago_syntax::ast::Sequence<'_, Statement<'_>>,
) -> Option<ClassInfo> {
    for stmt in statements {
        match stmt {
            Statement::Namespace(ns) => {
                let namespace = ns.name.map(|id| resolve_identifier(&id));
                for inner in ns.statements() {
                    if let Some(info) = extract_from_class_stmt(inner, namespace.as_deref()) {
                        return Some(info);
                    }
                }
            }
            _ => {
                if let Some(info) = extract_from_class_stmt(stmt, None) {
                    return Some(info);
                }
            }
        }
    }
    None
}

fn extract_from_class_stmt(stmt: &Statement<'_>, namespace: Option<&str>) -> Option<ClassInfo> {
    let Statement::Class(class) = stmt else {
        return None;
    };

    let name = class.name.value;
    let fqcn = match namespace {
        Some(ns) => format!("{ns}\\{name}"),
        None => name.to_owned(),
    };

    let parent = class
        .extends
        .as_ref()
        .and_then(|ext| ext.types.first())
        .map(|id| resolve_to_fqcn(&resolve_identifier(id), namespace));

    let mut property_count: u32 = 0;

    for member in &class.members {
        match member {
            ClassLikeMember::Property(_) => {
                property_count += 1;
            }
            ClassLikeMember::Method(method) => {
                if method.name.value == "__construct"
                    && matches!(method.body, MethodBody::Concrete(_))
                {
                    property_count += count_promoted_params(&method.parameter_list);
                }
            }
            _ => {}
        }
    }

    let estimated_bytes = estimate_object_bytes(property_count);

    Some(ClassInfo {
        fqcn,
        property_count,
        parent,
        estimated_bytes,
    })
}

#[allow(clippy::cast_possible_truncation)]
fn count_promoted_params(param_list: &mago_syntax::ast::FunctionLikeParameterList<'_>) -> u32 {
    param_list
        .parameters
        .iter()
        .filter(|p| {
            p.modifiers.iter().any(|m| {
                matches!(
                    m,
                    mago_syntax::ast::Modifier::Public(_)
                        | mago_syntax::ast::Modifier::Protected(_)
                        | mago_syntax::ast::Modifier::Private(_)
                        | mago_syntax::ast::Modifier::Readonly(_)
                )
            })
        })
        .count()
        .min(u32::MAX as usize) as u32
}

/// Estimates PHP object memory using measured PHP 8.4 numbers:
/// base `zend_object` ~72 bytes + 16 bytes per property zval slot.
#[must_use]
pub fn estimate_object_bytes(property_count: u32) -> u64 {
    ZEND_OBJECT_BASE + u64::from(property_count) * ZVAL_SLOT
}

fn resolve_identifier(id: &Identifier<'_>) -> String {
    match id {
        Identifier::FullyQualified(fq) => fq.value.trim_start_matches('\\').to_owned(),
        Identifier::Qualified(q) => q.value.to_owned(),
        Identifier::Local(l) => l.value.to_owned(),
    }
}

fn resolve_to_fqcn(raw: &str, namespace: Option<&str>) -> String {
    if raw.contains('\\') {
        return raw.to_owned();
    }
    match namespace {
        Some(ns) => format!("{ns}\\{raw}"),
        None => raw.to_owned(),
    }
}

fn parse_classmap(path: &Path, project_root: &Path) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return map;
    };

    // Lines like: 'Vendor\Class' => $vendorDir . '/vendor/path/Class.php',
    //         or: 'App\Class' => $baseDir . '/src/Class.php',
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(fqcn_start) = trimmed.find('\'') else {
            continue;
        };
        let rest = &trimmed[fqcn_start + 1..];
        let Some(fqcn_end) = rest.find('\'') else {
            continue;
        };
        let fqcn = &rest[..fqcn_end];

        let Some(path_start) = rest.find("'/") else {
            continue;
        };
        let path_rest = &rest[path_start + 1..];
        let Some(path_end) = path_rest.find('\'') else {
            continue;
        };
        let rel_path = &path_rest[..path_end];

        let file_path = if trimmed.contains("$vendorDir") {
            project_root
                .join("vendor")
                .join(rel_path.trim_start_matches('/'))
        } else {
            project_root.join(rel_path.trim_start_matches('/'))
        };

        map.insert(fqcn.replace("\\\\", "\\"), file_path);
    }

    map
}

fn parse_psr4(path: &Path, project_root: &Path) -> Vec<(String, PathBuf)> {
    let mut entries = Vec::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return entries;
    };

    // Lines like: 'Vendor\Package\' => array($vendorDir . '/vendor/package/src'),
    for line in content.lines() {
        let trimmed = line.trim();
        let Some(ns_start) = trimmed.find('\'') else {
            continue;
        };
        let rest = &trimmed[ns_start + 1..];
        let Some(ns_end) = rest.find('\'') else {
            continue;
        };
        let namespace = rest[..ns_end].replace("\\\\", "\\");

        let Some(path_start) = rest.find("'/") else {
            continue;
        };
        let path_rest = &rest[path_start + 1..];
        let Some(path_end) = path_rest.find('\'') else {
            continue;
        };
        let rel_path = &path_rest[..path_end];

        let dir = if trimmed.contains("$vendorDir") {
            project_root
                .join("vendor")
                .join(rel_path.trim_start_matches('/'))
        } else {
            project_root.join(rel_path.trim_start_matches('/'))
        };

        entries.push((namespace, dir));
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_php(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn counts_explicit_properties() {
        let tmp = tempfile::tempdir().unwrap();
        write_php(
            tmp.path(),
            "Foo.php",
            r#"<?php
namespace App;
class Foo {
    private string $name;
    protected int $count;
    public bool $active;
}
"#,
        );

        let info = introspect_class(&tmp.path().join("Foo.php")).unwrap();
        assert_eq!(info.fqcn, "App\\Foo");
        assert_eq!(info.property_count, 3);
        assert_eq!(info.estimated_bytes, 72 + 3 * 16);
    }

    #[test]
    fn counts_promoted_constructor_params() {
        let tmp = tempfile::tempdir().unwrap();
        write_php(
            tmp.path(),
            "Bar.php",
            r#"<?php
namespace App;
class Bar {
    public function __construct(
        private readonly string $name,
        private int $age,
        string $notPromoted,
    ) {}
}
"#,
        );

        let info = introspect_class(&tmp.path().join("Bar.php")).unwrap();
        assert_eq!(info.property_count, 2);
    }

    #[test]
    fn detects_parent_class() {
        let tmp = tempfile::tempdir().unwrap();
        write_php(
            tmp.path(),
            "Child.php",
            r#"<?php
namespace App;
class Child extends \App\Base {
    private $extra;
}
"#,
        );

        let info = introspect_class(&tmp.path().join("Child.php")).unwrap();
        assert_eq!(info.parent.as_deref(), Some("App\\Base"));
        assert_eq!(info.property_count, 1);
    }

    #[test]
    fn estimate_zero_props() {
        assert_eq!(estimate_object_bytes(0), 72);
    }

    #[test]
    fn estimate_ten_props() {
        assert_eq!(estimate_object_bytes(10), 72 + 160);
    }
}
