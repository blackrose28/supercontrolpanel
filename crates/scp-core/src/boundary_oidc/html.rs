//! Minimal, dependency-free HTML form extraction for IdP login pages.
//!
//! Only what the OIDC form driver needs: `<form>` action/method, `<input>` attributes, entity
//! decoding and the page `<title>`. It is not a general HTML parser; unknown markup is ignored.

/// One `<input>` element inside a form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormInput {
    pub name: String,
    pub kind: String,
    pub value: String,
    pub autocomplete: String,
    pub id: String,
    pub checked: bool,
}

/// One `<form>` element with the inputs that follow it before the next `</form>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Form {
    pub action: String,
    pub method: String,
    pub inputs: Vec<FormInput>,
}

/// Parses every `<form>` in `html`; inputs outside any form are ignored.
pub fn parse_forms(html: &str) -> Vec<Form> {
    let cleaned = strip_inert_blocks(html);
    let mut forms: Vec<Form> = Vec::new();
    let mut open = false;
    for tag in scan_tags(&cleaned) {
        match (tag.name.as_str(), tag.closing) {
            ("form", false) => {
                forms.push(Form {
                    action: tag.attr("action").unwrap_or_default().to_string(),
                    method: tag
                        .attr("method")
                        .map(str::to_ascii_lowercase)
                        .filter(|m| !m.is_empty())
                        .unwrap_or_else(|| "get".to_string()),
                    inputs: Vec::new(),
                });
                open = true;
            }
            ("form", true) => open = false,
            ("input", false) if open => {
                if let Some(form) = forms.last_mut() {
                    form.inputs.push(FormInput {
                        name: tag.attr("name").unwrap_or_default().to_string(),
                        kind: tag
                            .attr("type")
                            .map(str::to_ascii_lowercase)
                            .unwrap_or_else(|| "text".to_string()),
                        value: tag.attr("value").unwrap_or_default().to_string(),
                        autocomplete: tag.attr("autocomplete").unwrap_or_default().to_string(),
                        id: tag.attr("id").unwrap_or_default().to_string(),
                        checked: tag.attr("checked").is_some(),
                    });
                }
            }
            _ => {}
        }
    }
    forms
}

/// Returns the decoded, whitespace-collapsed `<title>` text, if any.
pub fn page_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let open = lower.find("<title")?;
    let body_start = open + lower[open..].find('>')? + 1;
    let body_end = body_start + lower[body_start..].find("</title")?;
    let text = decode_entities(&html[body_start..body_end]);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// Decodes the HTML entities that appear in attribute values (`&amp;`, `&#x3d;`, ...).
pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        match decode_one_entity(rest) {
            Some((ch, used)) => {
                out.push(ch);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_one_entity(s: &str) -> Option<(char, usize)> {
    let semi = s.find(';').filter(|&i| i <= 10)?;
    let name = &s[1..semi];
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse::<u32>().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((ch, semi + 1))
}

struct Tag {
    name: String,
    closing: bool,
    attrs: Vec<(String, String)>,
}

impl Tag {
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Removes comments and the bodies of `<script>`/`<style>` so markup inside them is ignored.
fn strip_inert_blocks(html: &str) -> String {
    let mut text = remove_between(html, "<!--", "-->");
    for tag in ["script", "style"] {
        text = remove_element(&text, tag);
    }
    text
}

fn remove_between(s: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        match rest[start + open.len()..].find(close) {
            Some(end) => rest = &rest[start + open.len() + end + close.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn remove_element(s: &str, tag: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let mut out = String::with_capacity(s.len());
    let mut pos = 0usize;
    while let Some(rel) = lower[pos..].find(&open) {
        let start = pos + rel;
        let after = lower.as_bytes().get(start + open.len()).copied();
        if !matches!(after, Some(b' ' | b'>' | b'\t' | b'\n' | b'\r' | b'/')) {
            out.push_str(&s[pos..start + open.len()]);
            pos = start + open.len();
            continue;
        }
        out.push_str(&s[pos..start]);
        let Some(close_rel) = lower[start..].find(&close) else {
            return out;
        };
        let close_start = start + close_rel;
        pos = lower[close_start..]
            .find('>')
            .map_or(s.len(), |i| close_start + i + 1);
    }
    out.push_str(&s[pos..]);
    out
}

fn scan_tags(html: &str) -> Vec<Tag> {
    let bytes = html.as_bytes();
    let mut tags = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let closing = bytes.get(j) == Some(&b'/');
        if closing {
            j += 1;
        }
        let name_start = j;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'-') {
            j += 1;
        }
        if j == name_start {
            i += 1;
            continue;
        }
        let name = html[name_start..j].to_ascii_lowercase();
        let (end, attrs) = parse_attrs(html, j);
        tags.push(Tag {
            name,
            closing,
            attrs,
        });
        i = end;
    }
    tags
}

/// Parses attributes starting at `from` until the closing `>`; returns the index after it.
fn parse_attrs(html: &str, from: usize) -> (usize, Vec<(String, String)>) {
    let bytes = html.as_bytes();
    let mut attrs = Vec::new();
    let mut i = from;
    while i < bytes.len() {
        match bytes[i] {
            b'>' => return (i + 1, attrs),
            b if b.is_ascii_whitespace() || b == b'/' => i += 1,
            _ => {
                let key_start = i;
                while i < bytes.len()
                    && !bytes[i].is_ascii_whitespace()
                    && !matches!(bytes[i], b'=' | b'>' | b'/')
                {
                    i += 1;
                }
                let key = html[key_start..i].to_ascii_lowercase();
                i = skip_whitespace(bytes, i);
                let mut value = String::new();
                if bytes.get(i) == Some(&b'=') {
                    i = skip_whitespace(bytes, i + 1);
                    (value, i) = read_attr_value(html, i);
                }
                attrs.push((key, value));
            }
        }
    }
    (bytes.len(), attrs)
}

fn skip_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Reads a quoted or bare attribute value at `i`, returning it decoded plus the next index.
fn read_attr_value(html: &str, i: usize) -> (String, usize) {
    let bytes = html.as_bytes();
    match bytes.get(i) {
        Some(&q) if q == b'"' || q == b'\'' => {
            let start = i + 1;
            let end = html[start..]
                .find(q as char)
                .map_or(html.len(), |p| start + p);
            (
                decode_entities(&html[start..end]),
                (end + 1).min(html.len()),
            )
        }
        _ => {
            let mut end = i;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() && bytes[end] != b'>' {
                end += 1;
            }
            (decode_entities(&html[i..end]), end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_named_and_numeric_entities() {
        assert_eq!(
            decode_entities("a&amp;b=1&amp;c=&quot;x&quot;&#39;&#x3D;&lt;&gt;"),
            "a&b=1&c=\"x\"'=<>"
        );
        assert_eq!(decode_entities("&unknown; &amp"), "&unknown; &amp");
    }

    #[test]
    fn parses_keycloak_login_form() {
        let html = r#"<!DOCTYPE html><html><head><title>Sign in to Corp</title>
<script>var x = '<form action="/evil">';</script></head><body>
<!-- <form action="/commented"> -->
<form id="kc-form-login" onsubmit="login.disabled = true; return true;"
  action="https://idp.example.test/realms/r/login-actions/authenticate?session_code=abc&amp;execution=e1&amp;tab_id=t"
  method="post">
  <input id="username" name="username" value="" type="text" autofocus autocomplete="off"/>
  <input id="password" name="password" type="password" autocomplete="off" />
  <input type="hidden" id="id-hidden-input" name="credentialId" />
  <input tabindex="4" name="login" id="kc-login" type="submit" value="Sign In"/>
</form></body></html>"#;
        let forms = parse_forms(html);
        assert_eq!(forms.len(), 1, "script/comment forms must be ignored");
        let f = &forms[0];
        assert_eq!(
            f.action,
            "https://idp.example.test/realms/r/login-actions/authenticate?session_code=abc&execution=e1&tab_id=t"
        );
        assert_eq!(f.method, "post");
        let names: Vec<_> = f.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["username", "password", "credentialId", "login"]);
        assert_eq!(f.inputs[1].kind, "password");
        assert_eq!(f.inputs[3].value, "Sign In");
        assert_eq!(
            page_title("<html><head><title>\n Sign   in &amp; go </title>"),
            Some("Sign in & go".to_string())
        );
    }

    #[test]
    fn attribute_parsing_handles_quotes_case_and_bare_values() {
        let html =
            "<FORM ACTION='/a?x=1&amp;y=2' Method=POST><INPUT Type=TEXT NAME=otp value='1 2'>\
                    <input type=checkbox name=rememberMe checked></FORM><input name=orphan>";
        let forms = parse_forms(html);
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].action, "/a?x=1&y=2");
        assert_eq!(forms[0].method, "post");
        assert_eq!(
            forms[0].inputs.len(),
            2,
            "orphan input outside form ignored"
        );
        assert_eq!(forms[0].inputs[0].kind, "text");
        assert_eq!(forms[0].inputs[0].name, "otp");
        assert_eq!(forms[0].inputs[0].value, "1 2");
        assert!(forms[0].inputs[1].checked);
    }

    #[test]
    fn default_method_is_get_and_missing_action_is_empty() {
        let forms = parse_forms("<form><input name=a></form>");
        assert_eq!(forms[0].method, "get");
        assert_eq!(forms[0].action, "");
    }
}
