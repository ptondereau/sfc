use crate::model::{Container, ServiceId};

use super::{AnalysisPass, Finding, Impact, Severity};

pub struct DeadRoutesPass;

impl AnalysisPass for DeadRoutesPass {
    fn name(&self) -> &'static str {
        "dead_routes"
    }

    fn run(&self, container: &Container) -> Vec<Finding> {
        let mut findings = vec![];

        for route in &container.routes {
            let controller_class = route
                .controller
                .split("::")
                .next()
                .unwrap_or(&route.controller);

            let key = ServiceId::new(controller_class);
            let exists = container.services.contains_key(&key);
            let aliased = container.aliases.contains_key(&key);

            if !exists && !aliased {
                findings.push(Finding {
                    pass: self.name(),
                    severity: Severity::Warning,
                    message: format!(
                        "route `{}` references controller `{}` which is not a registered service",
                        route.name, controller_class
                    ),
                    service_id: None,
                    file: None,
                    impact: Impact::None,
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

    #[test]
    fn route_with_existing_controller_ok() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(Service {
            id: ServiceId::new("App\\Controller\\BlogController"),
            class: "App\\Controller\\BlogController".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });
        c.routes.push(RouteDefinition {
            name: "blog_index".to_owned(),
            path: "/blog".to_owned(),
            controller: "App\\Controller\\BlogController::index".to_owned(),
            methods: vec!["GET".to_owned()],
        });
        let findings = DeadRoutesPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn route_with_missing_controller_flagged() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.routes.push(RouteDefinition {
            name: "missing_route".to_owned(),
            path: "/missing".to_owned(),
            controller: "App\\Controller\\MissingController::index".to_owned(),
            methods: vec!["GET".to_owned()],
        });
        let findings = DeadRoutesPass.run(&c);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("MissingController"));
    }

    #[test]
    fn route_with_aliased_controller_ok() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(Service {
            id: ServiceId::new("template_controller"),
            class: "Symfony\\Bundle\\FrameworkBundle\\Controller\\TemplateController".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![],
        });
        c.aliases.insert(
            ServiceId::new("Symfony\\Bundle\\FrameworkBundle\\Controller\\TemplateController"),
            ServiceId::new("template_controller"),
        );
        c.routes.push(RouteDefinition {
            name: "homepage".to_owned(),
            path: "/".to_owned(),
            controller:
                "Symfony\\Bundle\\FrameworkBundle\\Controller\\TemplateController::templateAction"
                    .to_owned(),
            methods: vec![],
        });
        let findings = DeadRoutesPass.run(&c);
        assert!(findings.is_empty());
    }

    #[test]
    fn no_routes_no_findings() {
        let c = Container::new(PathBuf::from("/tmp"));
        let findings = DeadRoutesPass.run(&c);
        assert!(findings.is_empty());
    }
}
