use std::collections::HashMap;

use crate::model::Container;

use super::introspect::{self, ClassInfo, ClassResolver};
use super::{AnalysisPass, Finding, Impact, Severity};

const FALLBACK_BYTES: u64 = 256;

pub struct ContainerWeightPass {
    resolver: ClassResolver,
}

impl ContainerWeightPass {
    #[must_use]
    pub fn new(project_root: &std::path::Path) -> Self {
        Self {
            resolver: ClassResolver::from_project(project_root),
        }
    }
}

impl AnalysisPass for ContainerWeightPass {
    fn name(&self) -> &'static str {
        "container_weight"
    }

    fn run(&self, container: &Container) -> Vec<Finding> {
        let mut class_cache: HashMap<String, Option<ClassInfo>> = HashMap::new();
        let mut total: u64 = 0;
        let mut service_weights: Vec<(&str, u64, u32)> = vec![];

        for node_idx in container.graph.node_indices() {
            let service = &container.graph[node_idx];

            let (bytes, props) = if service.class.is_empty() {
                (FALLBACK_BYTES, 0)
            } else {
                resolve_with_parents(&service.class, &self.resolver, &mut class_cache)
            };

            total += bytes;
            service_weights.push((&service.id.0, bytes, props));
        }

        service_weights.sort_by(|a, b| b.1.cmp(&a.1));

        let introspected = service_weights
            .iter()
            .filter(|(_, _, props)| *props > 0)
            .count();

        let mut findings = vec![Finding {
            pass: self.name(),
            severity: Severity::Info,
            message: format!(
                "container has {} services, estimated memory: {:.1} KB ({introspected} classes introspected)",
                container.service_count(),
                {
                    #[allow(clippy::cast_precision_loss)]
                    let kb = total as f64 / 1024.0;
                    kb
                }
            ),
            service_id: None,
            file: None,
            span: None,
            impact: Impact::Memory {
                estimated_bytes: total,
            },
            fix: None,
        }];

        for &(id, weight, props) in service_weights.iter().take(5) {
            findings.push(Finding {
                pass: self.name(),
                severity: Severity::Info,
                message: if props > 0 {
                    format!("service `{id}`: ~{weight} bytes ({props} properties)")
                } else {
                    format!("service `{id}`: ~{weight} bytes")
                },
                service_id: Some(crate::model::ServiceId::new(id)),
                file: None,
                span: None,
                impact: Impact::Memory {
                    estimated_bytes: weight,
                },
                fix: None,
            });
        }

        findings
    }
}

fn resolve_with_parents(
    fqcn: &str,
    resolver: &ClassResolver,
    cache: &mut HashMap<String, Option<ClassInfo>>,
) -> (u64, u32) {
    let mut total_props: u32 = 0;
    let mut current = Some(fqcn.to_owned());

    while let Some(ref class_name) = current {
        if !cache.contains_key(class_name) {
            let info = resolver
                .resolve(class_name)
                .and_then(|path| introspect::introspect_class(&path));
            cache.insert(class_name.clone(), info);
        }

        match cache.get(class_name) {
            Some(Some(info)) => {
                total_props += info.property_count;
                current = info.parent.clone();
            }
            _ => break,
        }
    }

    if total_props > 0 {
        (introspect::estimate_object_bytes(total_props), total_props)
    } else {
        (FALLBACK_BYTES, 0)
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::path::{Path, PathBuf};

    fn make_service(id: &str, class: &str) -> Service {
        Service {
            id: ServiceId::new(id),
            class: class.to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        }
    }

    #[test]
    fn weight_reports_total() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("a", "A"));
        c.add_service(make_service("b", "B"));
        // No real project root, so resolver won't find files → fallback
        let pass = ContainerWeightPass::new(Path::new("/nonexistent"));
        let findings = pass.run(&c);
        assert!(!findings.is_empty());
        assert!(findings[0].message.contains("2 services"));
    }

    #[test]
    fn weight_reports_top_services() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        for i in 0..10 {
            c.add_service(make_service(&format!("svc_{i}"), &format!("Class{i}")));
        }
        let pass = ContainerWeightPass::new(Path::new("/nonexistent"));
        let findings = pass.run(&c);
        assert!(findings.len() <= 6);
    }

    #[test]
    fn fallback_for_unresolvable_class() {
        let (bytes, props) = resolve_with_parents(
            "NonExistent\\Class",
            &ClassResolver::from_project(Path::new("/nonexistent")),
            &mut HashMap::new(),
        );
        assert_eq!(bytes, FALLBACK_BYTES);
        assert_eq!(props, 0);
    }
}
