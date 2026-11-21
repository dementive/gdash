//! Title clean-up and fuzzy matching helpers.

/// Turns a No-Intro / Redump / folder name into a display title:
/// `Elder Scrolls III, The - Morrowind (USA)` → `The Elder Scrolls III - Morrowind`.
pub fn clean_title(raw: &str) -> String {
    let mut s = raw.to_string();
    for ext in [".xiso.iso", ".iso", ".xex", ".zar", ".cso"] {
        if s.to_ascii_lowercase().ends_with(ext) {
            s.truncate(s.len() - ext.len());
        }
    }
    s = strip_groups(&s);
    s = strip_version_suffix(&s);
    s = s.replace('_', " ");
    s = move_article(&s);
    collapse_spaces(&s)
}

/// Removes `( … )` and `[ … ]` groups.
pub fn strip_groups(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    collapse_spaces(&out)
}

/// `Prodeus v1.0.2` → `Prodeus`.
fn strip_version_suffix(s: &str) -> String {
    let trimmed = s.trim_end();
    if let Some(idx) = trimmed.rfind(' ') {
        let last = &trimmed[idx + 1..];
        let looks_like_version = last.len() > 1
            && (last.starts_with('v') || last.starts_with('V'))
            && last[1..].chars().all(|c| c.is_ascii_digit() || c == '.')
            && last[1..].chars().any(|c| c.is_ascii_digit());
        if looks_like_version {
            return trimmed[..idx].to_string();
        }
    }
    trimmed.to_string()
}

/// `Elder Scrolls III, The - Morrowind` → `The Elder Scrolls III - Morrowind`.
fn move_article(s: &str) -> String {
    for article in ["The", "A", "An"] {
        let pat = format!(", {article}");
        if let Some(idx) = s.find(&pat) {
            let after = &s[idx + pat.len()..];
            if after.is_empty() || after.starts_with(' ') || after.starts_with(':') {
                return format!("{article} {}{}", &s[..idx], after);
            }
        }
    }
    s.to_string()
}

fn collapse_spaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lowercase alphanumerics only, with roman numerals II–X and "&" normalised.
pub fn normalize(s: &str) -> String {
    let words: Vec<String> = s
        .split(|c: char| !c.is_alphanumeric() && c != '&')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let lw = w.to_lowercase();
            match lw.as_str() {
                "ii" => "2".into(),
                "iii" => "3".into(),
                "iv" => "4".into(),
                "v" => "5".into(),
                "vi" => "6".into(),
                "vii" => "7".into(),
                "viii" => "8".into(),
                "ix" => "9".into(),
                "x" => "10".into(),
                "&" => "and".into(),
                _ => lw.replace('&', "and"),
            }
        })
        .collect();
    words.concat()
}

/// First character of each word: `Earth Defense Force 5` → `edf5`.
pub fn acronym(s: &str) -> String {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter_map(|w| w.chars().next())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Length of the longest common subsequence.
pub fn lcs(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for &ca in &a {
        for (j, &cb) in b.iter().enumerate() {
            cur[j + 1] = if ca == cb { prev[j] + 1 } else { prev[j + 1].max(cur[j]) };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Symmetric similarity in 0..=1 between two titles.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalize(a), normalize(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let l = lcs(&a, &b) as f32;
    2.0 * l / (a.len() + b.len()) as f32
}

/// Filesystem / id friendly slug.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// libretro-thumbnails file name rules: `&*/:`<>?\|"` become `_`.
pub fn libretro_name(s: &str) -> String {
    s.chars()
        .map(|c| if "&*/:`<>?\\|\"".contains(c) { '_' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_no_intro_names() {
        assert_eq!(
            clean_title("Elder Scrolls III, The - Morrowind - Game of the Year Edition (USA).xiso.iso"),
            "The Elder Scrolls III - Morrowind - Game of the Year Edition"
        );
        assert_eq!(
            clean_title("Sid Meier's Pirates! (USA, Europe) (En,Fr,De,Es,It).xiso.iso"),
            "Sid Meier's Pirates!"
        );
        assert_eq!(clean_title("Castle Crashers (World) (XBLA)"), "Castle Crashers");
        assert_eq!(clean_title("Prodeus v1.0.2"), "Prodeus");
        assert_eq!(clean_title("Star Wars - Knights of the Old Republic (USA) (Rev 1)"),
            "Star Wars - Knights of the Old Republic");
    }

    #[test]
    fn normalizes_and_matches() {
        assert_eq!(normalize("Left 4 Dead 2"), "left4dead2");
        assert_eq!(normalize("Galactic Civilizations IV"), "galacticcivilizations4");
        assert_eq!(acronym("Earth Defense Force 5"), "edf5");
        assert!(similarity("HELLDIVERS™ 2", "Helldivers 2") > 0.95);
        assert!(similarity("Tiny Glade", "Total War: PHARAOH") < 0.5);
    }

    #[test]
    fn slugs_and_libretro() {
        assert_eq!(slug("Orcs Must Die! 3"), "orcs-must-die-3");
        assert_eq!(libretro_name("Tom & Jerry: War?"), "Tom _ Jerry_ War_");
    }
}
