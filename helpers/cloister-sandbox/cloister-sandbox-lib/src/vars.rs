//! Safe environment-style variable expansion for $VARNAME tokens.

use std::collections::HashMap;

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_lowercase() || b == b'_'
}

fn is_ident(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

/// Expand $VARNAME occurrences in the template using the provided map.
/// Unknown variables are left intact (e.g., "$UNKNOWN").
pub fn expand_vars(template: &str, vars: &HashMap<String, String>) -> String {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            if start < bytes.len() && is_ident_start(bytes[start]) {
                let mut j = start + 1;
                while j < bytes.len() && is_ident(bytes[j]) {
                    j += 1;
                }
                let key = &template[start..j];
                if let Some(val) = vars.get(key) {
                    out.push_str(val);
                } else {
                    out.push('$');
                    out.push_str(key);
                }
                i = j;
                continue;
            }
        }
        let ch = template[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_vars() {
        let mut vars = HashMap::new();
        vars.insert("HOME".to_string(), "/home/user".to_string());
        vars.insert("DIR_HASH".to_string(), "abc123".to_string());

        assert_eq!(
            expand_vars("$HOME/.config/$DIR_HASH", &vars),
            "/home/user/.config/abc123"
        );
    }

    #[test]
    fn leaves_unknown_vars_intact() {
        let vars = HashMap::new();
        assert_eq!(expand_vars("$UNKNOWN/path", &vars), "$UNKNOWN/path");
    }

    #[test]
    fn handles_overlapping_var_names() {
        let mut vars = HashMap::new();
        vars.insert("HOME".to_string(), "/home/user".to_string());
        vars.insert("SANDBOX_HOME".to_string(), "/home/ubuntu".to_string());

        assert_eq!(
            expand_vars("$SANDBOX_HOME/.cache:$HOME/.cache", &vars),
            "/home/ubuntu/.cache:/home/user/.cache"
        );
    }

    #[test]
    fn ignores_dollar_without_identifier() {
        let mut vars = HashMap::new();
        vars.insert("HOME".to_string(), "/home/user".to_string());
        assert_eq!(expand_vars("$$$HOME", &vars), "$$/home/user");
    }

    #[test]
    fn handles_utf8_paths() {
        let mut vars = HashMap::new();
        vars.insert("HOME".to_string(), "/home/user".to_string());
        assert_eq!(
            expand_vars("$HOME/données", &vars),
            "/home/user/données"
        );
    }
}
