use petgraph::Direction;

use crate::model::{Container, ServiceId, Visibility};

use super::{AnalysisPass, Finding, Impact, Severity};

fn is_controller(class: &str) -> bool {
    class.contains("\\Controller\\")
}

pub struct DeadServicesPass;

impl AnalysisPass for DeadServicesPass {
    fn name(&self) -> &'static str {
        "dead_services"
    }

    fn run(&self, container: &Container) -> Vec<Finding> {
        let mut findings = vec![];
        let alias_targets: std::collections::HashSet<&ServiceId> =
            container.aliases.values().collect();

        for (service_id, &node_idx) in &container.services {
            let service = &container.graph[node_idx];

            if service.visibility == Visibility::Public {
                continue;
            }

            if service.has_role() {
                continue;
            }

            if is_controller(&service.class) {
                continue;
            }

            if container.kernel_referenced.contains(service_id) {
                continue;
            }

            let in_degree = container
                .graph
                .neighbors_directed(node_idx, Direction::Incoming)
                .count();

            let is_alias_target = alias_targets.contains(service_id);

            if in_degree == 0 && !is_alias_target {
                findings.push(Finding {
                    pass: self.name(),
                    severity: Severity::Warning,
                    message: format!(
                        "service `{}` ({}) is never injected",
                        service_id, service.class
                    ),
                    service_id: Some(service_id.clone()),
                    file: service.factory_file.clone(),
                    impact: Impact::Memory {
                        estimated_bytes: 512,
                    },
                });
            }
        }

        findings
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;
    use crate::model::ServiceRole;
    use crate::model::*;
    use std::path::PathBuf;

    fn make_service(id: &str, class: &str, vis: Visibility) -> Service {
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
    fn detects_dead_private_service() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("a", "A", Visibility::Private));
        c.add_service(make_service("b", "B", Visibility::Private));
        c.add_dependency(
            &ServiceId::new("b"),
            &ServiceId::new("a"),
            EdgeKind::Constructor,
        );
        let findings = DeadServicesPass.run(&c);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].service_id.as_ref().unwrap().0, "b");
    }

    #[test]
    fn public_service_never_dead() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("pub", "Pub", Visibility::Public));
        let findings = DeadServicesPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn service_with_role_not_dead() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        let mut svc = make_service("listener", "MyListener", Visibility::Private);
        svc.roles.push(ServiceRole::EventListener {
            event: "kernel.request".to_owned(),
            method: "onRequest".to_owned(),
            priority: 0,
        });
        c.add_service(svc);
        let findings = DeadServicesPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn voter_role_not_dead() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        let mut svc = make_service("voter", "App\\Security\\PostVoter", Visibility::Private);
        svc.roles.push(ServiceRole::Voter);
        c.add_service(svc);
        let findings = DeadServicesPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn alias_target_not_dead() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("mailer", "Mailer", Visibility::Private));
        c.aliases
            .insert(ServiceId::new("MailerInterface"), ServiceId::new("mailer"));
        let findings = DeadServicesPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn controller_class_not_dead() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service(
            "App\\Controller\\BlogController",
            "App\\Controller\\BlogController",
            Visibility::Private,
        ));
        let findings = DeadServicesPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn referenced_service_not_dead() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("a", "A", Visibility::Private));
        c.add_service(make_service("b", "B", Visibility::Public));
        c.add_dependency(
            &ServiceId::new("b"),
            &ServiceId::new("a"),
            EdgeKind::Constructor,
        );
        let findings = DeadServicesPass.run(&c);
        assert!(findings.is_empty());
    }
}
