use std::fs;
use std::path::Path;

use petgraph::Direction;
use petgraph::visit::EdgeRef;
use sfc::model::{EdgeKind, ServiceId, Visibility};
use sfc::parser::parse_container;

fn setup_container(dir: &Path) {
    let container_dir = dir.join("ContainerABC123");
    fs::create_dir_all(&container_dir).unwrap();

    let padding = " ".repeat(5000);
    let main_php = format!(
        r#"<?php

namespace ContainerABC123;

use Symfony\Component\DependencyInjection\Container;

class App_KernelProdContainer extends Container
{{
    public function __construct()
    {{
        $this->methodMap = [
            'event_dispatcher' => 'getEventDispatcherService',
            'request_stack' => 'getRequestStackService',
        ];
        $this->fileMap = [
            'App\\Service\\Mailer' => 'getMailerService',
            '.service_locator.test' => 'get_ServiceLocator_TestService',
        ];
        $this->aliases = [
            'Symfony\\Component\\EventDispatcher\\EventDispatcherInterface' => 'event_dispatcher',
        ];
    }}
}}
{padding}
"#
    );
    fs::write(container_dir.join("App_KernelProdContainer.php"), main_php).unwrap();

    fs::write(
        container_dir.join("getEventDispatcherService.php"),
        r#"<?php

namespace ContainerABC123;

class getEventDispatcherService extends App_KernelProdContainer
{
    public static function do($container, $lazyLoad = true)
    {
        $instance = new \Symfony\Component\EventDispatcher\EventDispatcher();

        $instance->addListener('kernel.request', fn () => ($container->privates['App\Service\Mailer'] ?? $container->load('getMailerService')));

        return $container->services['event_dispatcher'] = $instance;
    }
}
"#,
    )
    .unwrap();

    fs::write(
        container_dir.join("getRequestStackService.php"),
        r#"<?php

namespace ContainerABC123;

class getRequestStackService extends App_KernelProdContainer
{
    public static function do($container, $lazyLoad = true)
    {
        return $container->services['request_stack'] = $instance = new \Symfony\Component\HttpFoundation\RequestStack();
    }
}
"#,
    )
    .unwrap();

    fs::write(
        container_dir.join("get_ServiceLocator_TestService.php"),
        r#"<?php

namespace ContainerABC123;

class get_ServiceLocator_TestService extends App_KernelProdContainer
{
    public static function do($container, $lazyLoad = true)
    {
        return $container->privates['.service_locator.test'] = new \Symfony\Component\DependencyInjection\Argument\ServiceLocator(
            $container->getService ??= $container->getService(...),
            [
                'request_stack' => ['privates', '.service_locator.xyz', 'getRequestStackService', true],
            ],
            [
                'request_stack' => '?',
            ]
        );
    }
}
"#,
    )
    .unwrap();

    fs::write(
        container_dir.join("getMailerService.php"),
        r#"<?php

namespace ContainerABC123;

class getMailerService extends App_KernelProdContainer
{
    public static function do($container, $lazyLoad = true)
    {
        return $container->privates['App\Service\Mailer'] = new \App\Service\Mailer(
            ($container->services['event_dispatcher'] ?? self::getEventDispatcherService($container))
        );
    }
}
"#,
    )
    .unwrap();
}

#[test]
fn parse_container_extracts_services() {
    let dir = tempfile::tempdir().unwrap();
    setup_container(dir.path());

    let container = parse_container(dir.path()).unwrap();

    assert_eq!(container.service_count(), 4);
}

#[test]
fn parse_container_extracts_aliases() {
    let dir = tempfile::tempdir().unwrap();
    setup_container(dir.path());

    let container = parse_container(dir.path()).unwrap();

    assert_eq!(container.aliases.len(), 1);
    assert_eq!(
        container.aliases.get(&ServiceId::new(
            "Symfony\\Component\\EventDispatcher\\EventDispatcherInterface"
        )),
        Some(&ServiceId::new("event_dispatcher"))
    );
}

#[test]
fn parse_container_sets_visibility() {
    let dir = tempfile::tempdir().unwrap();
    setup_container(dir.path());

    let container = parse_container(dir.path()).unwrap();

    let ed_idx = container
        .services
        .get(&ServiceId::new("event_dispatcher"))
        .unwrap();
    assert_eq!(container.graph[*ed_idx].visibility, Visibility::Public);

    let mailer_idx = container
        .services
        .get(&ServiceId::new("App\\Service\\Mailer"))
        .unwrap();
    assert_eq!(container.graph[*mailer_idx].visibility, Visibility::Private);
}

#[test]
fn parse_container_resolves_class_names() {
    let dir = tempfile::tempdir().unwrap();
    setup_container(dir.path());

    let container = parse_container(dir.path()).unwrap();

    let ed_idx = container
        .services
        .get(&ServiceId::new("event_dispatcher"))
        .unwrap();
    assert_eq!(
        container.graph[*ed_idx].class,
        "Symfony\\Component\\EventDispatcher\\EventDispatcher"
    );

    let rs_idx = container
        .services
        .get(&ServiceId::new("request_stack"))
        .unwrap();
    assert_eq!(
        container.graph[*rs_idx].class,
        "Symfony\\Component\\HttpFoundation\\RequestStack"
    );
}

#[test]
fn parse_container_creates_dependency_edges() {
    let dir = tempfile::tempdir().unwrap();
    setup_container(dir.path());

    let container = parse_container(dir.path()).unwrap();

    assert!(
        container.graph.edge_count() >= 1,
        "expected at least 1 dependency edge, got {}",
        container.graph.edge_count()
    );
}

#[test]
fn parse_container_fails_without_container_dir() {
    let dir = tempfile::tempdir().unwrap();

    let result = parse_container(dir.path());
    assert!(result.is_err());
}

#[test]
fn arrow_function_deps_are_tracked() {
    let dir = tempfile::tempdir().unwrap();
    setup_container(dir.path());

    let container = parse_container(dir.path()).unwrap();

    let ed_idx = *container
        .services
        .get(&ServiceId::new("event_dispatcher"))
        .expect("event_dispatcher service must exist");

    let mailer_idx = *container
        .services
        .get(&ServiceId::new("App\\Service\\Mailer"))
        .expect("App\\Service\\Mailer service must exist");

    let has_edge = container
        .graph
        .edges_directed(ed_idx, Direction::Outgoing)
        .any(|e| e.target() == mailer_idx && matches!(e.weight(), EdgeKind::Constructor));

    assert!(
        has_edge,
        "event_dispatcher should have an outgoing edge to App\\Service\\Mailer"
    );
}

#[test]
fn service_locator_deps_tracked() {
    let dir = tempfile::tempdir().unwrap();
    setup_container(dir.path());

    let container = parse_container(dir.path()).unwrap();

    let locator_idx = *container
        .services
        .get(&ServiceId::new(".service_locator.test"))
        .expect(".service_locator.test service must exist");

    let rs_idx = *container
        .services
        .get(&ServiceId::new("request_stack"))
        .expect("request_stack service must exist");

    let has_edge = container
        .graph
        .edges_directed(locator_idx, Direction::Outgoing)
        .any(|e| e.target() == rs_idx && matches!(e.weight(), EdgeKind::Constructor));

    assert!(
        has_edge,
        ".service_locator.test should have an outgoing edge to request_stack"
    );
}
