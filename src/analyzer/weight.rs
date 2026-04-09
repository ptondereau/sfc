use crate::model::Container;

use super::{AnalysisPass, Finding, Impact, Severity};

const SERVICE_BASE_BYTES: u64 = 256;
const ARG_REF_BYTES: u64 = 64;
const TAG_BYTES: u64 = 128;
const PARAMETER_BYTES: u64 = 96;

pub struct ContainerWeightPass;

impl AnalysisPass for ContainerWeightPass {
    fn name(&self) -> &'static str {
        "container_weight"
    }

    fn run(&self, container: &Container) -> Vec<Finding> {
        let mut total: u64 = 0;
        let mut service_weights: Vec<(&str, u64)> = vec![];

        for node_idx in container.graph.node_indices() {
            let service = &container.graph[node_idx];
            let edge_count = container
                .graph
                .edges_directed(node_idx, petgraph::Direction::Outgoing)
                .count() as u64;

            let weight = SERVICE_BASE_BYTES
                + edge_count * ARG_REF_BYTES
                + service.tags.len() as u64 * TAG_BYTES;

            total += weight;
            service_weights.push((&service.id.0, weight));
        }

        total += container.parameters.len() as u64 * PARAMETER_BYTES;

        service_weights.sort_by(|a, b| b.1.cmp(&a.1));

        let mut findings = vec![Finding {
            pass: self.name(),
            severity: Severity::Info,
            message: format!(
                "container has {} services, estimated memory: {:.1} KB",
                container.service_count(),
                {
                    #[allow(clippy::cast_precision_loss)]
                    let kb = total as f64 / 1024.0;
                    kb
                }
            ),
            service_id: None,
            file: None,
            impact: Impact::Memory {
                estimated_bytes: total,
            },
            fix: None,
        }];

        for (id, weight) in service_weights.iter().take(5) {
            findings.push(Finding {
                pass: self.name(),
                severity: Severity::Info,
                message: format!("service `{id}`: ~{weight} bytes"),
                service_id: Some(crate::model::ServiceId::new(*id)),
                file: None,
                impact: Impact::Memory {
                    estimated_bytes: *weight,
                },
                fix: None,
            });
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
        let findings = ContainerWeightPass.run(&c);
        assert!(!findings.is_empty());
        assert!(findings[0].message.contains("2 services"));
    }

    #[test]
    fn weight_includes_parameters() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("a", "A"));
        c.parameters.insert(
            "kernel.debug".to_owned(),
            ParameterValue::Scalar("false".to_owned()),
        );
        let findings = ContainerWeightPass.run(&c);
        let total = match &findings[0].impact {
            Impact::Memory { estimated_bytes } => *estimated_bytes,
            _ => panic!("expected memory impact"),
        };
        assert!(total > SERVICE_BASE_BYTES);
    }

    #[test]
    fn weight_reports_top_services() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        for i in 0..10 {
            c.add_service(make_service(&format!("svc_{i}"), &format!("Class{i}")));
        }
        let findings = ContainerWeightPass.run(&c);
        assert!(findings.len() <= 6);
    }
}
