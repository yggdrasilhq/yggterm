use anyhow::Result;
use yggterm_core::SessionNode;

pub fn render_session_tree_text(root: &SessionNode) -> Result<String> {
    let mut out = String::new();
    out.push_str("Yggdrasil Terminal Session Tree\n");
    out.push_str("================================\n");
    render_node(root, 0, &mut out);
    Ok(out)
}

fn render_node(node: &SessionNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    out.push_str(&format!("{}- {}\n", indent, node.name));
    for child in &node.children {
        render_node(child, depth + 1, out);
    }
}
