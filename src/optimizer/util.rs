use std::path::Path;

#[must_use]
pub fn identify_factory_service(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for pattern in ["$container->privates['", "$container->services['"] {
        if let Some(start) = content.find(pattern) {
            let rest = &content[start + pattern.len()..];
            if let Some(end) = rest.find('\'') {
                return Some(rest[..end].to_owned());
            }
        }
    }
    None
}
