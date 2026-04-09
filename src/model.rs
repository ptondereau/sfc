use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub type ServiceGraph = DiGraph<Service, EdgeKind>;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ServiceId(pub String);

impl ServiceId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for ServiceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug)]
pub struct Service {
    pub id: ServiceId,
    pub class: String,
    pub factory_file: Option<PathBuf>,
    pub tags: Vec<Tag>,
    pub visibility: Visibility,
    #[allow(dead_code)]
    pub lazy: bool,
    pub roles: Vec<ServiceRole>,
}

impl Service {
    #[must_use]
    pub fn has_role(&self) -> bool {
        !self.roles.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    Constructor,
    #[allow(dead_code)]
    MethodCall,
    #[allow(dead_code)]
    TaggedIterator {
        tag: String,
    },
}

#[derive(Debug, Clone)]
pub struct Tag {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub attributes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum ServiceRole {
    EventListener {
        event: String,
        method: String,
        #[allow(dead_code)]
        priority: i32,
    },
    ConsoleCommand {
        #[allow(dead_code)]
        command_name: String,
    },
    Voter,
    #[allow(dead_code)]
    TwigComponent,
    #[allow(dead_code)]
    Normalizer,
}

#[derive(Debug)]
pub struct RouteDefinition {
    pub name: String,
    #[allow(dead_code)]
    pub path: String,
    pub controller: String,
    #[allow(dead_code)]
    pub methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ParameterValue {
    #[allow(dead_code)]
    Scalar(String),
    #[allow(dead_code)]
    Array(Vec<ParameterValue>),
    #[allow(dead_code)]
    Reference(ServiceId),
}

#[derive(Debug)]
pub struct Container {
    pub graph: ServiceGraph,
    pub services: HashMap<ServiceId, NodeIndex>,
    pub parameters: HashMap<String, ParameterValue>,
    pub aliases: HashMap<ServiceId, ServiceId>,
    #[allow(dead_code)]
    pub source_path: PathBuf,
    pub routes: Vec<RouteDefinition>,
    pub kernel_referenced: HashSet<ServiceId>,
}

impl Container {
    #[must_use]
    pub fn new(source_path: PathBuf) -> Self {
        Self {
            graph: DiGraph::new(),
            services: HashMap::new(),
            parameters: HashMap::new(),
            aliases: HashMap::new(),
            source_path,
            routes: vec![],
            kernel_referenced: HashSet::new(),
        }
    }

    #[must_use]
    pub fn add_service(&mut self, service: Service) -> NodeIndex {
        let id = service.id.clone();
        let idx = self.graph.add_node(service);
        self.services.insert(id, idx);
        idx
    }

    pub fn add_dependency(&mut self, from: &ServiceId, to: &ServiceId, kind: EdgeKind) {
        if let (Some(&from_idx), Some(&to_idx)) = (self.services.get(from), self.services.get(to)) {
            self.graph.add_edge(from_idx, to_idx, kind);
        }
    }

    #[must_use]
    pub fn service_count(&self) -> usize {
        self.graph.node_count()
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;

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
    fn add_service_to_container() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        let idx = c.add_service(make_service("app.foo", "App\\Foo", Visibility::Private));
        assert_eq!(c.service_count(), 1);
        assert_eq!(c.graph[idx].class, "App\\Foo");
    }

    #[test]
    fn add_dependency_creates_edge() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("a", "A", Visibility::Private));
        c.add_service(make_service("b", "B", Visibility::Private));
        c.add_dependency(
            &ServiceId::new("a"),
            &ServiceId::new("b"),
            EdgeKind::Constructor,
        );
        assert_eq!(c.graph.edge_count(), 1);
    }

    #[test]
    fn add_dependency_ignores_missing_services() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.add_service(make_service("a", "A", Visibility::Private));
        c.add_dependency(
            &ServiceId::new("a"),
            &ServiceId::new("missing"),
            EdgeKind::Constructor,
        );
        assert_eq!(c.graph.edge_count(), 0);
    }

    #[test]
    fn service_id_display() {
        let id = ServiceId::new("app.mailer");
        assert_eq!(format!("{id}"), "app.mailer");
    }

    #[test]
    fn aliases_resolve() {
        let mut c = Container::new(PathBuf::from("/tmp"));
        c.aliases
            .insert(ServiceId::new("MailerInterface"), ServiceId::new("mailer"));
        assert_eq!(
            c.aliases.get(&ServiceId::new("MailerInterface")),
            Some(&ServiceId::new("mailer"))
        );
    }
}
