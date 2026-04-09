use std::path::Path;

use crate::model::{Container, ServiceRole};

use super::{AnalysisPass, Finding, Impact, Severity};

const FRAMEWORK_EVENT_PREFIXES: &[&str] = &["kernel.", "console.", "security.", "Symfony\\"];

pub struct UnusedListenersPass {
    src_dir: Option<std::path::PathBuf>,
}

impl UnusedListenersPass {
    #[must_use]
    pub fn new(src_dir: &Path) -> Self {
        Self {
            src_dir: Some(src_dir.to_path_buf()),
        }
    }
}

impl AnalysisPass for UnusedListenersPass {
    fn name(&self) -> &'static str {
        "unused_listeners"
    }

    fn run(&self, container: &Container) -> Vec<Finding> {
        let Some(ref src_dir) = self.src_dir else {
            return vec![];
        };

        let src_content = collect_source_content(src_dir);

        let mut findings = vec![];

        for node_idx in container.graph.node_indices() {
            let service = &container.graph[node_idx];
            for role in &service.roles {
                let ServiceRole::EventListener { event, method, .. } = role else {
                    continue;
                };

                if is_framework_event(event) {
                    continue;
                }

                if !is_event_dispatched(event, &src_content) {
                    findings.push(Finding {
                        pass: self.name(),
                        severity: Severity::Warning,
                        message: format!(
                            "listener `{}::{}` listens to `{event}` which is never dispatched in src/",
                            service.id, method
                        ),
                        service_id: Some(service.id.clone()),
                        file: service.factory_file.clone(),
                        impact: Impact::Memory { estimated_bytes: 512 },
                    });
                }
            }
        }

        findings
    }
}

fn is_framework_event(event: &str) -> bool {
    FRAMEWORK_EVENT_PREFIXES
        .iter()
        .any(|prefix| event.starts_with(prefix))
}

fn is_event_dispatched(event: &str, src_content: &str) -> bool {
    let short_name = event.rsplit('\\').next().unwrap_or(event);
    src_content.contains(event) || src_content.contains(short_name)
}

fn collect_source_content(src_dir: &Path) -> String {
    let mut content = String::new();
    collect_dir_content(src_dir, &mut content);
    content
}

fn collect_dir_content(dir: &Path, content: &mut String) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_dir_content(&path, content);
        } else if path.extension().and_then(|e| e.to_str()) == Some("php")
            && let Ok(file_content) = std::fs::read_to_string(&path)
        {
            content.push_str(&file_content);
            content.push('\n');
        }
    }
}

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;

    #[test]
    fn framework_events_always_active() {
        assert!(is_framework_event("kernel.request"));
        assert!(is_framework_event("console.command"));
        assert!(is_framework_event("security.interactive_login"));
        assert!(is_framework_event(
            "Symfony\\Component\\Security\\Http\\Event\\LoginSuccessEvent"
        ));
        assert!(!is_framework_event("App\\Event\\CommentCreated"));
    }

    #[test]
    fn event_found_in_source() {
        let src = "class Foo { $this->dispatcher->dispatch(new CommentCreatedEvent()); }";
        assert!(is_event_dispatched("App\\Event\\CommentCreatedEvent", src));
    }

    #[test]
    fn event_not_found() {
        let src = "class Foo { return 42; }";
        assert!(!is_event_dispatched("App\\Event\\CustomEvent", src));
    }

    #[test]
    fn unused_listener_detected() {
        use crate::model::*;
        use std::path::PathBuf;

        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("Foo.php"), "<?php class Foo {}").unwrap();

        let mut container = Container::new(PathBuf::from("/tmp"));
        let svc = Service {
            id: ServiceId::new("my.listener"),
            class: "MyListener".to_owned(),
            factory_file: None,
            tags: vec![],
            visibility: Visibility::Private,
            lazy: false,
            roles: vec![ServiceRole::EventListener {
                event: "App\\Event\\NeverDispatched".to_owned(),
                method: "onEvent".to_owned(),
                priority: 0,
            }],
        };
        container.add_service(svc);

        let pass = UnusedListenersPass::new(&src_dir);
        let findings = pass.run(&container);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("NeverDispatched"));
    }
}
