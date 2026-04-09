use std::collections::{HashSet, VecDeque};

use petgraph::Direction;

use crate::model::{Container, ServiceId, Visibility};

/// Finds services whose factory files are unreachable from any runtime entry point.
///
/// Walks the parsed dependency graph starting from entry points (public services,
/// kernel-referenced services, alias targets, services with roles, controllers)
/// and follows all outgoing constructor/method-call edges transitively. Any service
/// with a factory file that is never visited is unreachable and safe to remove.
#[must_use]
pub fn find_unreachable_factories(container: &Container) -> HashSet<String> {
    let visited = bfs_from_entry_points(container);

    let mut unreachable = HashSet::new();
    for (service_id, &node_idx) in &container.services {
        let service = &container.graph[node_idx];
        if service.factory_file.is_some() && !visited.contains(service_id) {
            unreachable.insert(service_id.0.clone());
        }
    }

    unreachable
}

fn bfs_from_entry_points(container: &Container) -> HashSet<ServiceId> {
    let alias_targets: HashSet<&ServiceId> = container.aliases.values().collect();

    let mut visited: HashSet<ServiceId> = HashSet::new();
    let mut queue: VecDeque<ServiceId> = VecDeque::new();

    for (service_id, &node_idx) in &container.services {
        let service = &container.graph[node_idx];

        let is_entry = service.visibility == Visibility::Public
            || container.kernel_referenced.contains(service_id)
            || alias_targets.contains(service_id)
            || service.has_role()
            || is_controller(&service.class);

        if is_entry {
            visited.insert(service_id.clone());
            queue.push_back(service_id.clone());
        }
    }

    while let Some(current) = queue.pop_front() {
        let Some(&node_idx) = container.services.get(&current) else {
            continue;
        };

        for neighbor_idx in container
            .graph
            .neighbors_directed(node_idx, Direction::Outgoing)
        {
            let neighbor = &container.graph[neighbor_idx];
            if visited.insert(neighbor.id.clone()) {
                queue.push_back(neighbor.id.clone());
            }
        }
    }

    visited
}

fn is_controller(class: &str) -> bool {
    class.contains("\\Controller\\")
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::{EdgeKind, Service};

    fn make_service(id: &str, class: &str, vis: Visibility) -> Service {
        Service {
            id: ServiceId::new(id),
            class: class.to_owned(),
            factory_file: Some(PathBuf::from(format!("get{}Service.php", id))),
            tags: vec![],
            visibility: vis,
            lazy: false,
            roles: vec![],
        }
    }

    fn make_service_no_file(id: &str, class: &str, vis: Visibility) -> Service {
        Service {
            id: ServiceId::new(id),
            class: class.to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: vis,
            lazy: false,
            roles: vec![],
        }
    }

    #[test]
    fn public_services_and_deps_are_reachable() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("pub.svc", "PubService", Visibility::Public));
        c.add_service(make_service("dep.a", "DepA", Visibility::Private));
        c.add_service(make_service("dep.b", "DepB", Visibility::Private));
        c.add_service(make_service("orphan", "Orphan", Visibility::Private));
        c.add_dependency(
            &ServiceId::new("pub.svc"),
            &ServiceId::new("dep.a"),
            EdgeKind::Constructor,
        );
        c.add_dependency(
            &ServiceId::new("dep.a"),
            &ServiceId::new("dep.b"),
            EdgeKind::Constructor,
        );

        let unreachable = find_unreachable_factories(&c);
        assert!(!unreachable.contains("pub.svc"));
        assert!(!unreachable.contains("dep.a"));
        assert!(!unreachable.contains("dep.b"));
        assert!(
            unreachable.contains("orphan"),
            "orphan has no path from entry points"
        );
    }

    #[test]
    fn kernel_referenced_is_entry_point() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("kernel.dep", "KernelDep", Visibility::Private));
        c.kernel_referenced.insert(ServiceId::new("kernel.dep"));

        let unreachable = find_unreachable_factories(&c);
        assert!(!unreachable.contains("kernel.dep"));
    }

    #[test]
    fn alias_target_is_entry_point() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("mailer", "Mailer", Visibility::Private));
        c.aliases
            .insert(ServiceId::new("MailerInterface"), ServiceId::new("mailer"));

        let unreachable = find_unreachable_factories(&c);
        assert!(!unreachable.contains("mailer"));
    }

    #[test]
    fn services_without_factory_file_ignored() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service_no_file("inline", "Inline", Visibility::Public));
        c.add_service(make_service("orphan", "Orphan", Visibility::Private));

        let unreachable = find_unreachable_factories(&c);
        assert!(
            !unreachable.contains("inline"),
            "no factory file = nothing to remove"
        );
        assert!(unreachable.contains("orphan"));
    }

    #[test]
    fn transitive_deps_reached() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("root", "Root", Visibility::Public));
        c.add_service(make_service("mid", "Mid", Visibility::Private));
        c.add_service(make_service("leaf", "Leaf", Visibility::Private));
        c.add_service(make_service("island", "Island", Visibility::Private));

        c.add_dependency(
            &ServiceId::new("root"),
            &ServiceId::new("mid"),
            EdgeKind::Constructor,
        );
        c.add_dependency(
            &ServiceId::new("mid"),
            &ServiceId::new("leaf"),
            EdgeKind::Constructor,
        );

        let unreachable = find_unreachable_factories(&c);
        assert!(unreachable.is_empty() || unreachable == HashSet::from(["island".to_owned()]));
        assert!(!unreachable.contains("leaf"));
        assert!(unreachable.contains("island"));
    }

    #[test]
    fn controller_is_entry_point() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service(
            "App\\Controller\\HomeController",
            "App\\Controller\\HomeController",
            Visibility::Private,
        ));

        let unreachable = find_unreachable_factories(&c);
        assert!(!unreachable.contains("App\\Controller\\HomeController"));
    }
}
