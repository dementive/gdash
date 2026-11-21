//! Minimal parser for Valve's text KeyValues format (`.vdf` / `.acf`).

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Case-insensitive child lookup.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(items) => items
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    /// Follows a path of keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Vdf> {
        keys.iter().try_fold(self, |node, k| node.get(k))
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(items) => items,
            Vdf::Str(_) => &[],
        }
    }
}

/// Parses a whole document into a root object.
pub fn parse(text: &str) -> Vdf {
    let mut tokens = Tokens { chars: text.chars().peekable() };
    Vdf::Obj(parse_object(&mut tokens))
}

enum Tok {
    Str(String),
    Open,
    Close,
}

struct Tokens<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Iterator for Tokens<'_> {
    type Item = Tok;

    fn next(&mut self) -> Option<Tok> {
        loop {
            let c = *self.chars.peek()?;
            match c {
                c if c.is_whitespace() => {
                    self.chars.next();
                }
                '/' => {
                    // `//` line comment
                    self.chars.next();
                    if self.chars.peek() == Some(&'/') {
                        for c in self.chars.by_ref() {
                            if c == '\n' {
                                break;
                            }
                        }
                    }
                }
                '[' => {
                    // Conditional like [$WIN32]; ignored.
                    for c in self.chars.by_ref() {
                        if c == ']' {
                            break;
                        }
                    }
                }
                '{' => {
                    self.chars.next();
                    return Some(Tok::Open);
                }
                '}' => {
                    self.chars.next();
                    return Some(Tok::Close);
                }
                '"' => {
                    self.chars.next();
                    let mut s = String::new();
                    while let Some(c) = self.chars.next() {
                        match c {
                            '"' => break,
                            '\\' => match self.chars.next() {
                                Some('n') => s.push('\n'),
                                Some('t') => s.push('\t'),
                                Some(other) => s.push(other),
                                None => break,
                            },
                            c => s.push(c),
                        }
                    }
                    return Some(Tok::Str(s));
                }
                _ => {
                    // Unquoted token.
                    let mut s = String::new();
                    while let Some(&c) = self.chars.peek() {
                        if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                            break;
                        }
                        s.push(c);
                        self.chars.next();
                    }
                    return Some(Tok::Str(s));
                }
            }
        }
    }
}

fn parse_object(tokens: &mut Tokens) -> Vec<(String, Vdf)> {
    let mut items = Vec::new();
    while let Some(tok) = tokens.next() {
        let key = match tok {
            Tok::Str(s) => s,
            Tok::Close => break,
            Tok::Open => continue,
        };
        match tokens.next() {
            Some(Tok::Str(v)) => items.push((key, Vdf::Str(v))),
            Some(Tok::Open) => items.push((key, Vdf::Obj(parse_object(tokens)))),
            Some(Tok::Close) | None => break,
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest() {
        let doc = parse(
            r#""AppState"
{
	"appid"		"1086940"
	"name"		"Baldur's Gate 3"
	"installdir"		"Baldurs Gate 3"
	"UserConfig"
	{
		"language"		"english"
	}
}"#,
        );
        let app = doc.get("appstate").unwrap();
        assert_eq!(app.str("appid"), Some("1086940"));
        assert_eq!(app.str("NAME"), Some("Baldur's Gate 3"));
        assert_eq!(app.path(&["UserConfig"]).unwrap().str("language"), Some("english"));
    }

    #[test]
    fn handles_escapes_and_paths() {
        let doc = parse(r#""libraryfolders" { "0" { "path" "C:\\Program Files (x86)\\Steam" } }"#);
        let p = doc.path(&["libraryfolders", "0"]).unwrap().str("path");
        assert_eq!(p, Some(r"C:\Program Files (x86)\Steam"));
    }
}
