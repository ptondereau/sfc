use std::borrow::Cow;
use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::Path;

use bumpalo::Bump;
use mago_database::file::File;
use mago_syntax::ast::{Identifier, Statement};
use mago_syntax::parser::parse_file;

use super::{PhpClass, PreloadError};

/// # Errors
/// Returns `PreloadError::Io` if a directory cannot be read.
pub fn collect_classes(
    dirs: &[&Path],
    exclude_namespaces: &[String],
) -> Result<Vec<PhpClass>, PreloadError> {
    let mut classes = Vec::new();

    for dir in dirs {
        walk_directory(dir, &mut classes, exclude_namespaces)?;
    }

    Ok(classes)
}

/// Extract FQCNs from `use` statements in factory files (`get*Service.php`)
/// inside the container directory.
///
/// # Errors
/// Returns `PreloadError::Io` if the directory cannot be read.
pub fn extract_use_fqcns(container_dir: &Path) -> Result<HashSet<String>, PreloadError> {
    let entries = std::fs::read_dir(container_dir).map_err(|e| PreloadError::Io {
        path: container_dir.display().to_string(),
        source: e,
    })?;

    let mut fqcns = HashSet::new();

    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("get") || !name_str.ends_with("Service.php") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(fqcn) = parse_use_line(trimmed) {
                fqcns.insert(fqcn);
            }
        }
    }

    Ok(fqcns)
}

fn parse_use_line(line: &str) -> Option<String> {
    let rest = line.strip_prefix("use ")?;
    let fqcn = rest.strip_suffix(';')?.trim();
    if fqcn.is_empty() || fqcn.contains(' ') {
        return None;
    }
    Some(fqcn.trim_start_matches('\\').to_owned())
}

/// Collect PHP classes from `vendor/` whose FQCN appears in the provided set.
///
/// # Errors
/// Returns `PreloadError::Io` if the vendor directory cannot be read.
pub fn collect_vendor_classes_for_services<S: BuildHasher>(
    vendor_dir: &Path,
    used_fqcns: &HashSet<String, S>,
    exclude_namespaces: &[String],
) -> Result<Vec<PhpClass>, PreloadError> {
    let mut all_classes = Vec::new();
    walk_directory(vendor_dir, &mut all_classes, exclude_namespaces)?;

    let filtered: Vec<PhpClass> = all_classes
        .into_iter()
        .filter(|cls| used_fqcns.contains(&cls.fqcn))
        .collect();

    Ok(filtered)
}

fn walk_directory(
    dir: &Path,
    classes: &mut Vec<PhpClass>,
    exclude_namespaces: &[String],
) -> Result<(), PreloadError> {
    let entries = std::fs::read_dir(dir).map_err(|e| PreloadError::Io {
        path: dir.display().to_string(),
        source: e,
    })?;

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk_directory(&path, classes, exclude_namespaces)?;
        } else if path.extension().is_some_and(|ext| ext == "php") {
            collect_from_file(&path, classes, exclude_namespaces);
        }
    }

    Ok(())
}

fn collect_from_file(path: &Path, classes: &mut Vec<PhpClass>, exclude_namespaces: &[String]) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };

    let file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();

    let arena = Bump::new();
    let file = File::ephemeral(Cow::Owned(file_name), Cow::Owned(content));
    let program = parse_file(&arena, &file);

    if program.has_errors() {
        return;
    }

    for stmt in &program.statements {
        match stmt {
            Statement::Namespace(ns) => {
                let namespace = ns.name.map(|id| resolve_identifier(&id));
                for inner in ns.statements() {
                    if let Some(cls) = extract_declaration(inner, namespace.as_deref(), path)
                        && !is_excluded(&cls.fqcn, exclude_namespaces)
                    {
                        classes.push(cls);
                    }
                }
            }
            _ => {
                if let Some(cls) = extract_declaration(stmt, None, path)
                    && !is_excluded(&cls.fqcn, exclude_namespaces)
                {
                    classes.push(cls);
                }
            }
        }
    }
}

fn extract_declaration(
    stmt: &Statement<'_>,
    namespace: Option<&str>,
    path: &Path,
) -> Option<PhpClass> {
    match stmt {
        Statement::Class(class) => {
            let name = class.name.value;
            let fqcn = build_fqcn(namespace, name);

            let parent = class
                .extends
                .as_ref()
                .and_then(|ext| ext.types.first())
                .map(|id| resolve_to_fqcn(&resolve_identifier(id), namespace));

            let interfaces = class
                .implements
                .as_ref()
                .map(|imp| {
                    imp.types
                        .iter()
                        .map(|id| resolve_to_fqcn(&resolve_identifier(id), namespace))
                        .collect()
                })
                .unwrap_or_default();

            Some(PhpClass {
                fqcn,
                file_path: path.to_path_buf(),
                parent,
                interfaces,
            })
        }
        Statement::Interface(iface) => {
            let name = iface.name.value;
            let fqcn = build_fqcn(namespace, name);

            let interfaces = iface
                .extends
                .as_ref()
                .map(|ext| {
                    ext.types
                        .iter()
                        .map(|id| resolve_to_fqcn(&resolve_identifier(id), namespace))
                        .collect()
                })
                .unwrap_or_default();

            Some(PhpClass {
                fqcn,
                file_path: path.to_path_buf(),
                parent: None,
                interfaces,
            })
        }
        Statement::Trait(tr) => {
            let name = tr.name.value;
            let fqcn = build_fqcn(namespace, name);

            Some(PhpClass {
                fqcn,
                file_path: path.to_path_buf(),
                parent: None,
                interfaces: vec![],
            })
        }
        Statement::Enum(en) => {
            let name = en.name.value;
            let fqcn = build_fqcn(namespace, name);

            let interfaces = en
                .implements
                .as_ref()
                .map(|imp| {
                    imp.types
                        .iter()
                        .map(|id| resolve_to_fqcn(&resolve_identifier(id), namespace))
                        .collect()
                })
                .unwrap_or_default();

            Some(PhpClass {
                fqcn,
                file_path: path.to_path_buf(),
                parent: None,
                interfaces,
            })
        }
        _ => None,
    }
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

fn build_fqcn(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(ns) => format!("{ns}\\{name}"),
        None => name.to_owned(),
    }
}

fn is_excluded(fqcn: &str, exclude_namespaces: &[String]) -> bool {
    exclude_namespaces
        .iter()
        .any(|ns| fqcn.starts_with(ns.as_str()))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::TempDir;

    use super::*;

    fn write_php(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn collect_class_with_parent() {
        let tmp = TempDir::new().unwrap();
        write_php(
            tmp.path(),
            "Foo.php",
            r#"<?php
namespace App;

class Foo extends \App\Base implements \App\FooInterface {}
"#,
        );

        let dirs = [tmp.path()];
        let result = collect_classes(&dirs, &[]).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].fqcn, "App\\Foo");
        assert_eq!(result[0].parent.as_deref(), Some("App\\Base"));
        assert_eq!(result[0].interfaces, vec!["App\\FooInterface"]);
    }

    #[test]
    fn exclude_namespaces() {
        let tmp = TempDir::new().unwrap();
        write_php(
            tmp.path(),
            "Foo.php",
            "<?php\nnamespace App;\nclass Foo {}\n",
        );
        write_php(
            tmp.path(),
            "Bar.php",
            "<?php\nnamespace App\\Tests;\nclass Bar {}\n",
        );

        let dirs = [tmp.path()];
        let excludes = vec!["App\\Tests".to_owned()];
        let result = collect_classes(&dirs, &excludes).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].fqcn, "App\\Foo");
    }

    #[test]
    fn collect_interface_and_trait() {
        let tmp = TempDir::new().unwrap();
        write_php(
            tmp.path(),
            "Stuff.php",
            r#"<?php
namespace App;

interface FooInterface {}
trait FooTrait {}
"#,
        );

        let dirs = [tmp.path()];
        let result = collect_classes(&dirs, &[]).unwrap();

        assert_eq!(result.len(), 2);
        let fqcns: Vec<&str> = result.iter().map(|c| c.fqcn.as_str()).collect();
        assert!(fqcns.contains(&"App\\FooInterface"));
        assert!(fqcns.contains(&"App\\FooTrait"));
    }

    #[test]
    fn collect_enum() {
        let tmp = TempDir::new().unwrap();
        write_php(
            tmp.path(),
            "Status.php",
            "<?php\nnamespace App;\nenum Status { case Active; }\n",
        );

        let dirs = [tmp.path()];
        let result = collect_classes(&dirs, &[]).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].fqcn, "App\\Status");
    }

    #[test]
    fn extract_use_fqcns_from_factory_files() {
        let tmp = TempDir::new().unwrap();
        write_php(
            tmp.path(),
            "getMailerService.php",
            r#"<?php

use Symfony\Component\Mailer\Mailer;
use Symfony\Component\Mailer\Transport\Smtp\SmtpTransport;

return function () {
    return new Mailer(new SmtpTransport());
};
"#,
        );
        write_php(
            tmp.path(),
            "getCacheService.php",
            r#"<?php

use Symfony\Component\Cache\Adapter\TagAwareAdapter;

return function () {
    return new TagAwareAdapter();
};
"#,
        );
        write_php(tmp.path(), "notAFactory.php", "<?php\nuse App\\Ignored;\n");

        let fqcns = extract_use_fqcns(tmp.path()).unwrap();
        assert_eq!(fqcns.len(), 3);
        assert!(fqcns.contains("Symfony\\Component\\Mailer\\Mailer"));
        assert!(fqcns.contains("Symfony\\Component\\Mailer\\Transport\\Smtp\\SmtpTransport"));
        assert!(fqcns.contains("Symfony\\Component\\Cache\\Adapter\\TagAwareAdapter"));
        assert!(!fqcns.contains("App\\Ignored"));
    }

    #[test]
    fn parse_use_line_strips_leading_backslash() {
        assert_eq!(
            super::parse_use_line("use \\App\\Foo;"),
            Some("App\\Foo".to_owned())
        );
    }

    #[test]
    fn parse_use_line_rejects_non_use() {
        assert_eq!(super::parse_use_line("class Foo {}"), None);
    }

    #[test]
    fn parse_use_line_rejects_use_function() {
        assert_eq!(super::parse_use_line("use function strlen;"), None);
    }

    #[test]
    fn collect_vendor_classes_filters_by_fqcn() {
        let tmp = TempDir::new().unwrap();
        write_php(
            tmp.path(),
            "Mailer.php",
            "<?php\nnamespace Symfony\\Component\\Mailer;\nclass Mailer {}\n",
        );
        write_php(
            tmp.path(),
            "Logger.php",
            "<?php\nnamespace Monolog;\nclass Logger {}\n",
        );

        let mut used = std::collections::HashSet::new();
        used.insert("Symfony\\Component\\Mailer\\Mailer".to_owned());

        let result = collect_vendor_classes_for_services(tmp.path(), &used, &[]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].fqcn, "Symfony\\Component\\Mailer\\Mailer");
    }

    #[test]
    fn skip_files_with_parse_errors() {
        let tmp = TempDir::new().unwrap();
        write_php(tmp.path(), "Bad.php", "<?php\nclass { broken");
        write_php(
            tmp.path(),
            "Good.php",
            "<?php\nnamespace App;\nclass Good {}\n",
        );

        let dirs = [tmp.path()];
        let result = collect_classes(&dirs, &[]).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].fqcn, "App\\Good");
    }

    #[test]
    fn walks_subdirectories() {
        let tmp = TempDir::new().unwrap();
        write_php(
            tmp.path(),
            "sub/Deep.php",
            "<?php\nnamespace App\\Sub;\nclass Deep {}\n",
        );

        let dirs = [tmp.path()];
        let result = collect_classes(&dirs, &[]).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].fqcn, "App\\Sub\\Deep");
    }
}
