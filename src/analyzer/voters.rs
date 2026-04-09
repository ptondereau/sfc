use crate::model::{Container, ServiceRole};

use super::{AnalysisPass, Finding, Impact, Severity};

pub struct AlwaysLoadedVotersPass;

impl AnalysisPass for AlwaysLoadedVotersPass {
    fn name(&self) -> &'static str {
        "always_loaded_voters"
    }

    fn run(&self, container: &Container) -> Vec<Finding> {
        let mut findings = vec![];

        for node_idx in container.graph.node_indices() {
            let service = &container.graph[node_idx];
            let is_voter = service
                .roles
                .iter()
                .any(|r| matches!(r, ServiceRole::Voter));

            if is_voter && !service.lazy {
                findings.push(Finding {
                    pass: self.name(),
                    severity: Severity::Info,
                    message: format!(
                        "voter `{}` ({}) is loaded on every authorized request — consider making it lazy",
                        service.id, service.class
                    ),
                    service_id: Some(service.id.clone()),
                    file: service.factory_file.clone(),
                    impact: Impact::Startup { estimated_ms: 1 },
                    fix: Some(format!(
                        "# config/services.yaml\nservices:\n    {}:\n        lazy: true",
                        service.class
                    )),
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
    use crate::model::*;
    use std::path::PathBuf;

    fn make_voter(id: &str, class: &str, lazy: bool) -> Service {
        Service {
            id: ServiceId::new(id),
            class: class.to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy,
            roles: vec![ServiceRole::Voter],
        }
    }

    #[test]
    fn non_lazy_voter_flagged() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_voter("app.voter", "App\\Security\\PostVoter", false));
        let findings = AlwaysLoadedVotersPass.run(&c);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("PostVoter"));
    }

    #[test]
    fn lazy_voter_not_flagged() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_voter("app.voter", "App\\Security\\PostVoter", true));
        let findings = AlwaysLoadedVotersPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn non_voter_ignored() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(Service {
            id: ServiceId::new("app.service"),
            class: "App\\Service\\Foo".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });
        let findings = AlwaysLoadedVotersPass.run(&c);
        assert!(findings.is_empty());
    }
}
