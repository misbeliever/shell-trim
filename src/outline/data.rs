pub(super) fn outline_markdown(lines: &[&str]) -> Vec<String> {
    let mut headings: Vec<(usize, usize, &str)> = Vec::new();
    let mut in_fence = false;
    for (i, raw) in lines.iter().enumerate() {
        let t = raw.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if t.starts_with('#') {
            let level = t.bytes().take_while(|&b| b == b'#').count();
            if level <= 4 && t.as_bytes().get(level) == Some(&b' ') {
                headings.push((i, level, t.trim_end()));
            }
        }
    }

    let mut out = Vec::with_capacity(headings.len());
    for (idx, &(line_idx, level, text)) in headings.iter().enumerate() {
        let end_idx = headings
            .iter()
            .skip(idx + 1)
            .find(|&&(_, l, _)| l <= level)
            .map(|&(next_i, _, _)| next_i.saturating_sub(1))
            .unwrap_or_else(|| lines.len().saturating_sub(1));
        let range = format!("L{}-{}", line_idx + 1, end_idx + 1);
        out.push(format!("{range:>11}| {text}"));
    }
    out
}

pub(super) fn outline_json(lines: &[&str]) -> Vec<String> {
    // Emit top-level keys (indentation 1-4 spaces) with approximate ranges
    let mut keys: Vec<(usize, String)> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let indent = raw.len() - raw.trim_start().len();
        let trimmed = raw.trim();
        if (indent == 2 || indent == 4 || indent == 1)
            && trimmed.starts_with('"')
            && let Some(colon) = trimmed.find("\":")
        {
            let key = &trimmed[..colon + 1];
            let val_hint = trimmed[colon + 2..].trim();
            let preview = if val_hint.len() > 40 {
                format!("{}…", &val_hint[..37])
            } else {
                val_hint.to_string()
            };
            keys.push((i, format!("{key}: {preview}")));
        }
    }
    let mut out = Vec::with_capacity(keys.len());
    for (idx, (line_idx, label)) in keys.iter().enumerate() {
        let end_idx = keys
            .get(idx + 1)
            .map(|(next_i, _)| next_i.saturating_sub(1))
            .unwrap_or_else(|| lines.len().saturating_sub(1));
        let range = if end_idx > *line_idx {
            format!("L{}-{}", line_idx + 1, end_idx + 1)
        } else {
            format!("L{}", line_idx + 1)
        };
        out.push(format!("{range:>11}| {label}"));
    }
    out
}

pub(super) fn outline_yaml(lines: &[&str]) -> Vec<String> {
    let mut keys: Vec<(usize, String)> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        if raw.is_empty() || raw.starts_with(' ') || raw.starts_with('\t') || raw.starts_with('#') {
            continue;
        }
        if raw.starts_with("---") || raw.starts_with("...") {
            continue;
        }
        if let Some((k, v)) = raw.split_once(':') {
            let v = v.trim();
            let preview = if v.len() > 50 {
                format!("{}…", &v[..47])
            } else {
                v.to_string()
            };
            keys.push((i, format!("{k}: {preview}")));
        }
    }
    let mut out = Vec::with_capacity(keys.len());
    for (idx, (line_idx, label)) in keys.iter().enumerate() {
        let end_idx = keys
            .get(idx + 1)
            .map(|(next_i, _)| next_i.saturating_sub(1))
            .unwrap_or_else(|| lines.len().saturating_sub(1));
        let range = if end_idx > *line_idx {
            format!("L{}-{}", line_idx + 1, end_idx + 1)
        } else {
            format!("L{}", line_idx + 1)
        };
        out.push(format!("{range:>11}| {label}"));
    }
    out
}
